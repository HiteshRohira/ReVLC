//! Pure-Rust media metadata extraction and container indexing (MP4/MOV,
//! Matroska/WebM, and audio via `lofty`).
//!
//! Replaces `libvlc_media_parse_with_options` polling for all common formats
//! and builds per-track keyframe index tables (timestamps + byte offsets +
//! codec configuration) used by the Stage 3 warm-session preview decoder.

pub mod audio;
pub mod mkv;
pub mod mp4;

use std::fs::File;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::engine::{Chapter, MediaMeta};

/// Maximum single video packet size we will read from a container (32 MiB),
/// guarding against corrupt size fields causing unbounded allocations.
pub const MAX_PACKET_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    Hevc,
    Av1,
    Vp9,
    Other(String),
}

impl VideoCodec {
    pub fn is_hw_preview_supported(&self) -> bool {
        matches!(self, Self::H264 | Self::Hevc | Self::Av1 | Self::Vp9)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerKind {
    Mp4,
    Mkv,
}

/// One indexed video keyframe inside a container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyframeEntry {
    /// Presentation timestamp in milliseconds.
    pub pts_ms: i64,
    /// MP4: exact file byte offset of the sample.
    /// MKV: exact file byte offset of the containing Cluster.
    pub offset: u64,
    /// MP4: exact byte length of the sample. MKV: 0 (parsed from Cluster/Block).
    pub size: u32,
    /// MKV: optional relative offset from the Cluster's data payload start
    /// (`CueRelativePosition`), allowing direct jump to the Block.
    pub cluster_rel_offset: Option<u64>,
    /// MKV: optional 1-based block number within the Cluster (`CueBlockNumber`).
    pub block_number: Option<u64>,
}

/// Indexed video stream information & keyframe table.
#[derive(Clone, Debug)]
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
pub struct VideoTrackIndex {
    /// 0-based index among non-cover-art video tracks.
    #[allow(dead_code)]
    pub index: usize,
    /// Container track identifier (`tkhd` track_id or MKV `TrackNumber`).
    pub track_id: u64,
    pub container: ContainerKind,
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    /// Clockwise display rotation in degrees (0, 90, 180, 270).
    pub rotation: u16,
    /// Raw codec configuration box (`avcC`, `hvcC`, `av1C`, `vpcC`, or MKV `CodecPrivate`).
    pub extradata: Vec<u8>,
    /// MKV ContentEncodings header-stripping prefix bytes, if any.
    pub header_strip: Vec<u8>,
    /// Keyframes sorted by `pts_ms` ascending.
    pub keyframes: Vec<KeyframeEntry>,
}

impl VideoTrackIndex {
    /// Returns the index of the keyframe nearest to `target_ms`.
    pub fn nearest_keyframe_index(&self, target_ms: i64) -> Option<usize> {
        if self.keyframes.is_empty() {
            return None;
        }
        match self.keyframes.binary_search_by_key(&target_ms, |kf| kf.pts_ms) {
            Ok(i) => Some(i),
            Err(0) => Some(0),
            Err(i) if i >= self.keyframes.len() => Some(self.keyframes.len() - 1),
            Err(i) => {
                let prev = self.keyframes[i - 1].pts_ms;
                let next = self.keyframes[i].pts_ms;
                if (target_ms - prev) <= (next - target_ms) { Some(i - 1) } else { Some(i) }
            }
        }
    }
}

/// Full pure-Rust index of a container file.
#[derive(Clone, Debug, Default)]
pub struct IndexedMedia {
    pub meta: MediaMeta,
    pub chapters: Vec<Chapter>,
    pub video_tracks: Vec<VideoTrackIndex>,
}

/// Reusable reader that keeps one file handle open while reading keyframe
/// packets for an active preview session.
pub struct PacketReader {
    file: File,
    file_len: u64,
}

impl PacketReader {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file_len = file.metadata().map_err(|e| e.to_string())?.len();
        Ok(Self { file, file_len })
    }

    pub fn read_keyframe(&mut self, track: &VideoTrackIndex, kf: &KeyframeEntry) -> Result<Vec<u8>, String> {
        match track.container {
            ContainerKind::Mp4 => {
                let size = kf.size as usize;
                if size == 0 || size > MAX_PACKET_BYTES {
                    return Err(format!("invalid MP4 sample size: {size}"));
                }
                let end = kf.offset.checked_add(size as u64).ok_or("MP4 sample offset overflow")?;
                if end > self.file_len {
                    return Err("MP4 sample extends past EOF".into());
                }
                self.file.seek(SeekFrom::Start(kf.offset)).map_err(|e| e.to_string())?;
                let mut buf = vec![0u8; size];
                self.file.read_exact(&mut buf).map_err(|e| e.to_string())?;
                Ok(buf)
            }
            ContainerKind::Mkv => mkv::read_keyframe_packet(&mut self.file, self.file_len, track, kf),
        }
    }
}

const AUDIO_LOFTY_EXTS: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "opus", "spx", "wav", "aif", "aiff", "aifc", "m4a", "m4b", "aac", "ape", "wv", "mpc",
];
const MP4_EXTS: &[&str] = &["mp4", "m4v", "mov", "qt", "3gp", "3g2", "f4v"];
const MKV_EXTS: &[&str] = &["mkv", "webm", "mka", "mks"];

fn ext_lower(path: &Path) -> Option<String> {
    path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase())
}

/// Extracts metadata (tags, duration, embedded artwork) in pure Rust when
/// the file's format is supported. Returns `None` for exotic formats so the
/// caller can fall back to `Engine::parse`.
pub fn parse_metadata(path: &Path, art_dir: &Path) -> Option<MediaMeta> {
    let ext = ext_lower(path)?;
    if AUDIO_LOFTY_EXTS.contains(&ext.as_str()) {
        if let Ok(meta) = audio::parse_audio(path, art_dir) {
            return Some(meta);
        }
        // Some .m4a files might still parse via our MP4 box walker.
        if matches!(ext.as_str(), "m4a" | "m4b") {
            return mp4::parse_meta(path, art_dir).ok();
        }
        return None;
    }
    if MP4_EXTS.contains(&ext.as_str()) {
        return mp4::parse_meta(path, art_dir).ok();
    }
    if MKV_EXTS.contains(&ext.as_str()) {
        return mkv::parse_meta(path, art_dir).ok();
    }
    None
}

/// Parses container headers, metadata, chapters, and video keyframe index
/// tables for MP4/MOV and Matroska/WebM files without spawning a decoder.
pub fn index_video(path: &Path) -> Result<IndexedMedia, String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut header = [0u8; 12];
    let n = file.read(&mut header).map_err(|e| e.to_string())?;
    drop(file);
    if n >= 4 && header[0..4] == [0x1A, 0x45, 0xDF, 0xA3] {
        return mkv::index(path);
    }
    if n >= 8 && matches!(&header[4..8], b"ftyp" | b"moov" | b"mdat" | b"free" | b"wide" | b"skip" | b"pnot") {
        return mp4::index(path);
    }
    // Fallback to extension sniff if magic header was unusual.
    match ext_lower(path).as_deref() {
        Some(e) if MP4_EXTS.contains(&e) => mp4::index(path),
        Some(e) if MKV_EXTS.contains(&e) => mkv::index(path),
        _ => Err("unsupported container for pure-Rust indexing".into()),
    }
}

/// Saves embedded artwork bytes to a deterministic content-hashed file in
/// `art_dir`, returning its path. Reuses an existing file with the same hash
/// safely across concurrent metadata scanner workers.
pub(crate) fn write_embedded_art(art_dir: &Path, data: &[u8]) -> Option<String> {
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    if data.len() < 16 || data.len() > 20 * 1024 * 1024 {
        return None;
    }
    let ext = if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "png"
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else {
        return None;
    };
    let mut h = DefaultHasher::new();
    data.hash(&mut h);
    let digest = h.finish();
    let _ = std::fs::create_dir_all(art_dir);
    let out: PathBuf = art_dir.join(format!("emb-{digest:016x}.{ext}"));
    if !out.exists() {
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = art_dir.join(format!("emb-{digest:016x}-{}-{seq}.tmp", std::process::id()));
        if std::fs::write(&tmp, data).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return out.exists().then(|| out.to_string_lossy().into_owned());
        }
        if std::fs::rename(&tmp, &out).is_err() {
            let _ = std::fs::remove_file(&tmp);
            // Another worker may have published the identical content-hashed file concurrently.
            if !out.exists() {
                return None;
            }
        }
    }
    Some(out.to_string_lossy().into_owned())
}

/// Parses a track/disc tag like "3" or "3/12" into `Some(3)`.
pub(crate) fn parse_number_tag(s: &str) -> Option<u32> {
    s.split('/').next()?.trim().parse::<u32>().ok().filter(|&n| n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_keyframe_picks_closest_timestamp() {
        let track = VideoTrackIndex {
            index: 0,
            track_id: 1,
            container: ContainerKind::Mp4,
            codec: VideoCodec::H264,
            width: 1920,
            height: 1080,
            rotation: 0,
            extradata: Vec::new(),
            header_strip: Vec::new(),
            keyframes: vec![
                KeyframeEntry { pts_ms: 0, offset: 0, size: 10, cluster_rel_offset: None, block_number: None },
                KeyframeEntry { pts_ms: 2000, offset: 100, size: 10, cluster_rel_offset: None, block_number: None },
                KeyframeEntry { pts_ms: 5000, offset: 200, size: 10, cluster_rel_offset: None, block_number: None },
            ],
        };
        assert_eq!(track.nearest_keyframe_index(-100), Some(0));
        assert_eq!(track.nearest_keyframe_index(0), Some(0));
        assert_eq!(track.nearest_keyframe_index(900), Some(0));
        assert_eq!(track.nearest_keyframe_index(1100), Some(1));
        assert_eq!(track.nearest_keyframe_index(3500), Some(1));
        assert_eq!(track.nearest_keyframe_index(3600), Some(2));
        assert_eq!(track.nearest_keyframe_index(99_000), Some(2));
    }

    #[test]
    fn corrupt_and_nonexistent_files_never_panic() {
        let dir = std::env::temp_dir().join(format!("revlc-container-corrupt-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let art = dir.join("art");

        let garbage_mp4 = dir.join("garbage.mp4");
        std::fs::write(&garbage_mp4, [0x00, 0x00, 0x00, 0x01, b'm', b'o', b'o', b'v', 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]).unwrap();
        assert!(index_video(&garbage_mp4).is_err());
        assert!(parse_metadata(&garbage_mp4, &art).is_none());

        let garbage_mkv = dir.join("garbage.mkv");
        std::fs::write(&garbage_mkv, [0x1A, 0x45, 0xDF, 0xA3, 0xFF, 0xFF, 0xFF, 0xFF, 0x18, 0x53, 0x80, 0x67, 0x00]).unwrap();
        assert!(index_video(&garbage_mkv).is_err());

        let empty_mp3 = dir.join("empty.mp3");
        std::fs::write(&empty_mp3, b"").unwrap();
        assert!(parse_metadata(&empty_mp3, &art).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn indexes_real_mp4_hevc_and_mkv_when_testmedia_present() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("testmedia");
        let art = std::env::temp_dir().join(format!("revlc-container-art-{}", std::process::id()));

        let mp4 = root.join("test-1080p.mp4");
        if mp4.exists() {
            let idx = index_video(&mp4).expect("index 1080p mp4");
            assert!((idx.meta.duration - 60_000).abs() < 1000, "dur={}", idx.meta.duration);
            assert_eq!(idx.video_tracks.len(), 1);
            let t = &idx.video_tracks[0];
            assert_eq!(t.codec, VideoCodec::H264);
            assert_eq!((t.width, t.height), (1920, 1080));
            assert!(!t.extradata.is_empty(), "avcC extradata");
            assert!(!t.keyframes.is_empty(), "keyframes indexed");
            let mut pr = PacketReader::open(&mp4).unwrap();
            let pkt = pr.read_keyframe(t, &t.keyframes[0]).unwrap();
            assert!(!pkt.is_empty());
        }

        let hevc = root.join("promo/Webb - Cassiopeia A.mp4");
        if hevc.exists() {
            let idx = index_video(&hevc).expect("index 4k hevc mp4");
            assert_eq!(idx.video_tracks[0].codec, VideoCodec::Hevc);
            assert_eq!((idx.video_tracks[0].width, idx.video_tracks[0].height), (3840, 2160));
            assert!(!idx.video_tracks[0].extradata.is_empty(), "hvcC extradata");
            assert!(!idx.video_tracks[0].keyframes.is_empty());
        }

        let mkv = root.join("Tears of Steel.mkv");
        if mkv.exists() {
            let idx = index_video(&mkv).expect("index mkv");
            assert!(idx.meta.duration > 100_000, "mkv dur={}", idx.meta.duration);
            assert!(!idx.video_tracks.is_empty());
            let t = &idx.video_tracks[0];
            assert_eq!(t.codec, VideoCodec::H264);
            assert!(!t.extradata.is_empty(), "CodecPrivate");
            assert!(!t.keyframes.is_empty(), "Cues keyframes");
            let mut pr = PacketReader::open(&mkv).unwrap();
            let pkt = pr.read_keyframe(t, &t.keyframes[0]).unwrap();
            assert!(!pkt.is_empty());
            let meta = parse_metadata(&mkv, &art).expect("mkv parse_metadata");
            assert!(meta.duration > 100_000);
        }

        let _ = std::fs::remove_dir_all(&art);
    }

    #[test]
    fn parses_audio_tags_duration_and_embedded_cover_with_lofty() {
        let ffmpeg = std::env::var_os("REVLC_TEST_FFMPEG")
            .map(PathBuf::from)
            .or_else(crate::keyframes::ffmpeg);
        let Some(ffmpeg) = ffmpeg else { return };
        let dir = std::env::temp_dir().join(format!("revlc-audio-meta-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let art_dir = dir.join("art");
        let cover = dir.join("cover.jpg");
        image::RgbImage::from_fn(40, 40, |x, y| image::Rgb([(x * 6) as u8, (y * 6) as u8, 128]))
            .save(&cover)
            .unwrap();

        let mp3 = dir.join("song.mp3");
        let ok = std::process::Command::new(&ffmpeg)
            .args([
                "-hide_banner", "-loglevel", "error", "-y",
                "-f", "lavfi", "-t", "3", "-i", "sine=frequency=440:sample_rate=44100",
                "-i", cover.to_str().unwrap(),
                "-map", "0:a", "-map", "1:v",
                "-c:a", "libmp3lame", "-b:a", "128k",
                "-c:v", "copy", "-disposition:v:0", "attached_pic",
                "-id3v2_version", "3",
                "-metadata", "title=Starlight",
                "-metadata", "artist=The Voyagers",
                "-metadata", "album=Orbit",
                "-metadata", "album_artist=The Voyagers",
                "-metadata", "genre=Electronic",
                "-metadata", "track=4/10",
                "-metadata", "disc=1/2",
                "-metadata", "date=2025",
                mp3.to_str().unwrap(),
            ])
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            let meta = parse_metadata(&mp3, &art_dir).expect("parse_metadata mp3");
            assert_eq!(meta.title.as_deref(), Some("Starlight"));
            assert_eq!(meta.artist.as_deref(), Some("The Voyagers"));
            assert_eq!(meta.album.as_deref(), Some("Orbit"));
            assert_eq!(meta.album_artist.as_deref(), Some("The Voyagers"));
            assert_eq!(meta.genre.as_deref(), Some("Electronic"));
            assert_eq!(meta.track, Some(4));
            assert_eq!(meta.disc, Some(1));
            assert_eq!(meta.date.as_deref(), Some("2025"));
            assert!((meta.duration - 3000).abs() < 500, "dur={}", meta.duration);
            let art_path = meta.artwork.expect("embedded cover extracted");
            assert!(Path::new(&art_path).exists());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_write_embedded_art_never_loses_artwork() {
        let base = std::env::temp_dir().join(format!("revlc-emb-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for round in 0..20u8 {
            let dir = base.join(format!("r{round}"));
            let mut fake_jpg = vec![0xFF, 0xD8, 0xFF, 0xE0, round];
            fake_jpg.resize(256, 0x42 ^ round);
            let data = std::sync::Arc::new(fake_jpg);
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    let dir = dir.clone();
                    let data = std::sync::Arc::clone(&data);
                    let barrier = std::sync::Arc::clone(&barrier);
                    std::thread::spawn(move || {
                        barrier.wait();
                        write_embedded_art(&dir, &data)
                    })
                })
                .collect();
            let results: Vec<Option<String>> = handles.into_iter().map(|h| h.join().unwrap()).collect();
            assert!(results.iter().all(|r| r.is_some()), "round {round}: {results:?}");
            let first = results[0].as_ref().unwrap();
            assert!(results.iter().all(|r| r.as_deref() == Some(first.as_str())));
            assert_eq!(std::fs::read(first).unwrap(), *data);
            let files: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            assert_eq!(files.len(), 1, "leftover temp files in round {round}: {files:?}");
            assert!(files[0].starts_with("emb-") && files[0].ends_with(".jpg"));
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
