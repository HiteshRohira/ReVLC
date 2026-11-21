//! Pure-Rust streaming ISO BMFF (MP4 / M4V / MOV / M4A) metadata and
//! keyframe index parser.
//!
//! Reads only the `moov` box hierarchy (never loading `mdat` media payloads),
//! extracting duration, iTunes `ilst` tags, embedded cover art, Nero `chpl`
//! chapters, video track descriptions (`avcC`/`hvcC`/`av1C`/`vpcC`), rotation
//! matrices, and exact `(pts_ms, file_offset, size)` tables for every keyframe.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::engine::{Chapter, MediaMeta};

use super::{ContainerKind, IndexedMedia, KeyframeEntry, MAX_PACKET_BYTES, VideoCodec, VideoTrackIndex, write_embedded_art};

/// Maximum `moov` or metadata atom size we will buffer in memory (32 MiB).
const MAX_MOOV_BYTES: u64 = 32 * 1024 * 1024;
/// Maximum entries in a single MP4 sample table (guards corrupt counts).
const MAX_TABLE_ENTRIES: usize = 2_000_000;
/// Aggregate sample-table budget across all tracks in one file.
const MAX_TOTAL_TABLE_ENTRIES: usize = 2_000_000;
/// Maximum video tracks indexed in one MP4 file.
const MAX_VIDEO_TRACKS: usize = 32;

/// Reads only metadata (tags, duration, embedded artwork) from an MP4/MOV file.
pub fn parse_meta(path: &Path, art_dir: &Path) -> Result<MediaMeta, String> {
    let parsed = parse_internal(path, Some(art_dir), false)?;
    Ok(parsed.meta)
}

/// Full index: metadata, chapters, and per-video-track keyframe index tables.
pub fn index(path: &Path) -> Result<IndexedMedia, String> {
    parse_internal(path, None, true)
}

fn parse_internal(path: &Path, art_dir: Option<&Path>, build_keyframes: bool) -> Result<IndexedMedia, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file_len = file.metadata().map_err(|e| e.to_string())?.len();
    if file_len < 8 {
        return Err("MP4 file too small".into());
    }
    let mut reader = BufReader::new(file);

    // Locate the top-level `moov` box by walking box headers only.
    let mut pos: u64 = 0;
    let mut saw_iso = false;
    let mut moov_range: Option<(u64, u64)> = None;
    let mut boxes_seen = 0usize;

    while pos + 8 <= file_len && boxes_seen < 4096 {
        boxes_seen += 1;
        reader.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        let mut hdr = [0u8; 8];
        if reader.read_exact(&mut hdr).is_err() {
            break;
        }
        let size32 = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as u64;
        let kind = [hdr[4], hdr[5], hdr[6], hdr[7]];
        let (header_len, box_size) = if size32 == 1 {
            if pos + 16 > file_len {
                break;
            }
            let mut ext = [0u8; 8];
            reader.read_exact(&mut ext).map_err(|e| e.to_string())?;
            let sz = u64::from_be_bytes(ext);
            if sz < 16 {
                return Err("corrupt MP4 64-bit box size".into());
            }
            (16u64, sz)
        } else if size32 == 0 {
            (8u64, file_len.saturating_sub(pos))
        } else if size32 < 8 {
            return Err("corrupt MP4 box size".into());
        } else {
            (8u64, size32)
        };

        let box_end = pos.checked_add(box_size).ok_or("MP4 box size overflow")?;
        if box_end > file_len {
            if &kind == b"moov" {
                return Err("MP4 moov box truncated past EOF".into());
            }
            break;
        }
        if matches!(&kind, b"ftyp" | b"moov" | b"mdat" | b"free" | b"wide" | b"skip") {
            saw_iso = true;
        }
        if &kind == b"moov" {
            let payload_start = pos + header_len;
            let payload_len = box_end.saturating_sub(payload_start);
            if payload_len == 0 || payload_len > MAX_MOOV_BYTES {
                return Err("MP4 moov box size invalid or exceeds safety limit".into());
            }
            moov_range = Some((payload_start, payload_len));
            break;
        }
        if box_end <= pos {
            break;
        }
        pos = box_end;
    }

    if !saw_iso {
        return Err("not an ISO BMFF/MP4 file".into());
    }
    let (moov_start, moov_len) = moov_range.ok_or("MP4 moov box not found")?;
    reader.seek(SeekFrom::Start(moov_start)).map_err(|e| e.to_string())?;
    let mut moov = vec![0u8; moov_len as usize];
    reader.read_exact(&mut moov).map_err(|e| e.to_string())?;

    let mut out = IndexedMedia::default();
    let mut movie_timescale: u32 = 1000;
    let mut video_track_ordinal = 0usize;
    let mut remaining_entries = MAX_TOTAL_TABLE_ENTRIES;
    let mut saw_unparsed_mdta = false;

    for (kind, payload) in BoxIter::new(&moov) {
        match &kind {
            b"mvhd" => {
                if let Some((ts, dur_ms)) = parse_mvhd(payload) {
                    movie_timescale = ts;
                    out.meta.duration = dur_ms;
                }
            }
            b"udta" => {
                parse_udta(payload, &mut out.meta, &mut out.chapters, art_dir, &mut saw_unparsed_mdta);
            }
            b"meta" => {
                parse_meta_box(payload, &mut out.meta, art_dir, &mut saw_unparsed_mdta);
            }
            b"trak" if build_keyframes && out.video_tracks.len() < MAX_VIDEO_TRACKS => {
                if let Some(track) = parse_video_trak(payload, movie_timescale, file_len, video_track_ordinal, &mut remaining_entries) {
                    video_track_ordinal += 1;
                    out.video_tracks.push(track);
                }
            }
            _ => {}
        }
    }

    // If a QuickTime `mdta` metadata table had unrecognized keys and yielded no
    // core tags, decline metadata fast-path so `Engine::parse` can read them.
    if !build_keyframes && saw_unparsed_mdta && out.meta.title.is_none() && out.meta.artist.is_none() && out.meta.album.is_none() {
        return Err("QuickTime mdta keys require fallback parser".into());
    }

    // Fill chapter durations from adjacent chapter start times when missing.
    for i in 0..out.chapters.len() {
        if out.chapters[i].duration <= 0 {
            let end = out.chapters.get(i + 1).map(|c| c.start).unwrap_or(out.meta.duration);
            out.chapters[i].duration = (end - out.chapters[i].start).max(0);
        }
    }

    Ok(out)
}

/// Zero-copy iterator over child ISO BMFF boxes in a byte slice.
struct BoxIter<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> BoxIter<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
}

impl<'a> Iterator for BoxIter<'a> {
    type Item = ([u8; 4], &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos + 8 > self.buf.len() {
            return None;
        }
        let size32 = u32::from_be_bytes(self.buf[self.pos..self.pos + 4].try_into().ok()?) as u64;
        let kind: [u8; 4] = self.buf[self.pos + 4..self.pos + 8].try_into().ok()?;
        let (hdr, size) = if size32 == 1 {
            if self.pos + 16 > self.buf.len() {
                return None;
            }
            let sz = u64::from_be_bytes(self.buf[self.pos + 8..self.pos + 16].try_into().ok()?);
            if sz < 16 {
                return None;
            }
            (16usize, usize::try_from(sz).ok()?)
        } else if size32 == 0 {
            (8usize, self.buf.len() - self.pos)
        } else if size32 < 8 {
            return None;
        } else {
            (8usize, size32 as usize)
        };
        let end = self.pos.checked_add(size)?;
        if end > self.buf.len() || end < self.pos + hdr {
            return None;
        }
        let payload = &self.buf[self.pos + hdr..end];
        self.pos = end;
        Some((kind, payload))
    }
}

fn be_u16(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

fn be_u32(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

fn be_i32(b: &[u8], off: usize) -> Option<i32> {
    Some(i32::from_be_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

fn be_u64(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(off..off + 8)?.try_into().ok()?))
}

fn be_i64(b: &[u8], off: usize) -> Option<i64> {
    Some(i64::from_be_bytes(b.get(off..off + 8)?.try_into().ok()?))
}

/// Parses `mvhd` returning `(timescale, duration_ms)`.
fn parse_mvhd(b: &[u8]) -> Option<(u32, i64)> {
    let version = *b.first()?;
    let (timescale, duration) = if version == 1 {
        (be_u32(b, 20)?, be_u64(b, 24)?)
    } else {
        (be_u32(b, 12)?, be_u32(b, 16)? as u64)
    };
    if timescale == 0 {
        return None;
    }
    let dur_ms = ((duration as u128).saturating_mul(1000) / timescale as u128).min(i64::MAX as u128) as i64;
    Some((timescale, dur_ms))
}

fn parse_udta(b: &[u8], meta: &mut MediaMeta, chapters: &mut Vec<Chapter>, art_dir: Option<&Path>, saw_unparsed_mdta: &mut bool) {
    for (kind, payload) in BoxIter::new(b) {
        match &kind {
            b"meta" => parse_meta_box(payload, meta, art_dir, saw_unparsed_mdta),
            b"chpl" => parse_chpl(payload, chapters),
            _ => {}
        }
    }
}

fn parse_meta_box(b: &[u8], meta: &mut MediaMeta, art_dir: Option<&Path>, saw_unparsed_mdta: &mut bool) {
    // ISO `meta` is a FullBox (4-byte version/flags); QuickTime sometimes omits those 4 bytes.
    let children = if b.len() >= 12 && (&b[8..12] == b"hdlr" || &b[8..12] == b"ilst" || &b[8..12] == b"keys") {
        &b[4..]
    } else {
        b
    };
    let mut mdta_keys: Vec<String> = Vec::new();
    let mut ilst_payload: Option<&[u8]> = None;
    for (kind, payload) in BoxIter::new(children) {
        match &kind {
            b"keys" => mdta_keys = parse_mdta_keys(payload),
            b"ilst" => ilst_payload = Some(payload),
            _ => {}
        }
    }
    if let Some(payload) = ilst_payload {
        if !mdta_keys.is_empty() {
            if !parse_mdta_ilst(payload, &mdta_keys, meta, art_dir) {
                *saw_unparsed_mdta = true;
            }
        } else {
            parse_ilst(payload, meta, art_dir);
        }
    }
}

/// Parses QuickTime `keys` box (list of `mdta` key names, 1-indexed from `ilst`).
fn parse_mdta_keys(b: &[u8]) -> Vec<String> {
    let Some(count) = be_u32(b, 4) else { return Vec::new() };
    let mut off = 8usize;
    let mut keys = Vec::with_capacity((count as usize).min(256));
    for _ in 0..count.min(256) {
        let Some(sz) = be_u32(b, off).map(|v| v as usize) else { break };
        if sz < 8 || off.checked_add(sz).is_none_or(|e| e > b.len()) {
            break;
        }
        let name = String::from_utf8_lossy(&b[off + 8..off + sz]).trim().to_ascii_lowercase();
        keys.push(name);
        off += sz;
    }
    keys
}

/// Parses QuickTime `mdta`-indexed `ilst` entries. Returns `true` if any known
/// metadata field was extracted.
fn parse_mdta_ilst(b: &[u8], keys: &[String], meta: &mut MediaMeta, art_dir: Option<&Path>) -> bool {
    let mut extracted = false;
    for (kind, payload) in BoxIter::new(b) {
        let idx = u32::from_be_bytes(kind) as usize;
        let Some(key_name) = idx.checked_sub(1).and_then(|i| keys.get(i)) else { continue };
        for (sub_kind, sub_payload) in BoxIter::new(payload) {
            if &sub_kind != b"data" || sub_payload.len() < 8 {
                continue;
            }
            let data_type = be_u32(sub_payload, 0).unwrap_or(0);
            let val = &sub_payload[8..];
            let text = || -> Option<String> {
                if data_type == 1 || data_type == 0 {
                    let s = String::from_utf8_lossy(val).trim().to_string();
                    (!s.is_empty()).then_some(s)
                } else {
                    None
                }
            };
            match key_name.as_str() {
                "com.apple.quicktime.title" | "com.apple.quicktime.displayname" => {
                    if let Some(v) = text() {
                        meta.title = meta.title.take().or(Some(v));
                        extracted = true;
                    }
                }
                "com.apple.quicktime.artist" | "com.apple.quicktime.author" => {
                    if let Some(v) = text() {
                        meta.artist = meta.artist.take().or(Some(v));
                        extracted = true;
                    }
                }
                "com.apple.quicktime.album" => {
                    if let Some(v) = text() {
                        meta.album = meta.album.take().or(Some(v));
                        extracted = true;
                    }
                }
                "com.apple.quicktime.creationdate" => {
                    if let Some(v) = text() {
                        meta.date = meta.date.take().or(Some(v));
                        extracted = true;
                    }
                }
                "com.apple.quicktime.genre" => {
                    if let Some(v) = text() {
                        meta.genre = meta.genre.take().or(Some(v));
                        extracted = true;
                    }
                }
                "com.apple.quicktime.artwork" if meta.artwork.is_none() => {
                    if let Some(dir) = art_dir
                        && let Some(p) = write_embedded_art(dir, val)
                    {
                        meta.artwork = Some(p);
                        extracted = true;
                    }
                }
                _ => {}
            }
        }
    }
    extracted
}

fn parse_ilst(b: &[u8], meta: &mut MediaMeta, art_dir: Option<&Path>) {
    for (kind, payload) in BoxIter::new(b) {
        // Each item in `ilst` holds one or more `data` boxes (8-byte type/locale header + value).
        for (sub_kind, sub_payload) in BoxIter::new(payload) {
            if &sub_kind != b"data" || sub_payload.len() < 8 {
                continue;
            }
            let data_type = be_u32(sub_payload, 0).unwrap_or(0);
            let val = &sub_payload[8..];
            let text = || -> Option<String> {
                if data_type == 1 || data_type == 0 {
                    let s = String::from_utf8_lossy(val).trim().to_string();
                    (!s.is_empty()).then_some(s)
                } else {
                    None
                }
            };
            match &kind {
                b"\xa9nam" => meta.title = meta.title.take().or_else(text),
                b"\xa9ART" => meta.artist = meta.artist.take().or_else(text),
                b"aART" => meta.album_artist = meta.album_artist.take().or_else(text),
                b"\xa9alb" => meta.album = meta.album.take().or_else(text),
                b"\xa9day" => meta.date = meta.date.take().or_else(text),
                b"\xa9gen" => meta.genre = meta.genre.take().or_else(text),
                b"tvsh" => meta.show = meta.show.take().or_else(text),
                b"tvsn" if val.len() >= 4 => {
                    let n = be_u32(val, 0).unwrap_or(0);
                    if n > 0 {
                        meta.season = Some(n);
                    }
                }
                b"tves" if val.len() >= 4 => {
                    let n = be_u32(val, 0).unwrap_or(0);
                    if n > 0 {
                        meta.episode = Some(n);
                    }
                }
                b"trkn" if val.len() >= 4 => {
                    let n = be_u16(val, 2).unwrap_or(0) as u32;
                    if n > 0 {
                        meta.track = Some(n);
                    }
                }
                b"disk" if val.len() >= 4 => {
                    let n = be_u16(val, 2).unwrap_or(0) as u32;
                    if n > 0 {
                        meta.disc = Some(n);
                    }
                }
                b"covr" if meta.artwork.is_none() => {
                    if let Some(dir) = art_dir {
                        meta.artwork = write_embedded_art(dir, val);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Parses Nero `chpl` chapters.
fn parse_chpl(b: &[u8], chapters: &mut Vec<Chapter>) {
    if b.len() < 5 {
        return;
    }
    let version = b[0];
    let (count, mut off) = if version == 1 {
        if b.len() < 9 {
            return;
        }
        (b[8] as usize, 9usize)
    } else {
        (b[4] as usize, 5usize)
    };
    for i in 0..count.min(1024) {
        if off + 9 > b.len() {
            break;
        }
        let ts_100ns = be_u64(b, off).unwrap_or(0);
        let name_len = b[off + 8] as usize;
        off += 9;
        if off + name_len > b.len() {
            break;
        }
        let name = String::from_utf8_lossy(&b[off..off + name_len]).trim().to_string();
        off += name_len;
        let start = (ts_100ns / 10_000).min(i64::MAX as u64) as i64;
        chapters.push(Chapter {
            start,
            duration: 0,
            name: if name.is_empty() { format!("Chapter {}", i + 1) } else { name },
        });
    }
}

/// Extracts display rotation (clockwise degrees: 0, 90, 180, 270) from a `tkhd`
/// 3×3 fixed-point transformation matrix.
fn rotation_from_matrix(b: &[u8], off: usize) -> u16 {
    let (Some(a), Some(b_val)) = (be_i32(b, off), be_i32(b, off + 4)) else {
        return 0;
    };
    let af = a as f64 / 65536.0;
    let bf = b_val as f64 / 65536.0;
    if af == 0.0 && bf == 0.0 {
        return 0;
    }
    let deg = bf.atan2(af).to_degrees().rem_euclid(360.0);
    (((deg + 45.0) / 90.0).floor() as u16 * 90) % 360
}

struct StscEntry {
    first_chunk: u32,
    samples_per_chunk: u32,
    sample_desc_index: u32,
}

fn parse_video_trak(
    b: &[u8],
    movie_timescale: u32,
    file_len: u64,
    ordinal: usize,
    remaining_entries: &mut usize,
) -> Option<VideoTrackIndex> {
    let mut track_id: u64 = 0;
    let mut rotation: u16 = 0;
    let mut tkhd_width: u32 = 0;
    let mut tkhd_height: u32 = 0;
    let mut elst_box: Option<&[u8]> = None;
    let mut mdia_slice: Option<&[u8]> = None;

    for (kind, payload) in BoxIter::new(b) {
        match &kind {
            b"tkhd" => {
                let version = *payload.first()?;
                if version == 1 {
                    track_id = be_u32(payload, 20)? as u64;
                    rotation = rotation_from_matrix(payload, 52);
                    tkhd_width = be_u32(payload, 88).unwrap_or(0) >> 16;
                    tkhd_height = be_u32(payload, 92).unwrap_or(0) >> 16;
                } else {
                    track_id = be_u32(payload, 12)? as u64;
                    rotation = rotation_from_matrix(payload, 40);
                    tkhd_width = be_u32(payload, 76).unwrap_or(0) >> 16;
                    tkhd_height = be_u32(payload, 80).unwrap_or(0) >> 16;
                }
            }
            b"edts" => {
                for (sub, p) in BoxIter::new(payload) {
                    if &sub == b"elst" {
                        elst_box = Some(p);
                    }
                }
            }
            b"mdia" => mdia_slice = Some(payload),
            _ => {}
        }
    }

    let mdia = mdia_slice?;
    let mut timescale: u32 = 0;
    let mut is_video = false;
    let mut minf_slice: Option<&[u8]> = None;

    for (kind, payload) in BoxIter::new(mdia) {
        match &kind {
            b"mdhd" => {
                let version = *payload.first()?;
                timescale = if version == 1 { be_u32(payload, 20)? } else { be_u32(payload, 12)? };
            }
            b"hdlr" => {
                if payload.get(8..12) == Some(b"vide") {
                    is_video = true;
                }
            }
            b"minf" => minf_slice = Some(payload),
            _ => {}
        }
    }

    if !is_video || timescale == 0 {
        return None;
    }

    let edit_offset_ms = elst_box.map_or(0, |p| parse_elst_offset(p, movie_timescale, timescale));

    let mut stbl_slice: Option<&[u8]> = None;
    for (kind, payload) in BoxIter::new(minf_slice?) {
        if &kind == b"stbl" {
            stbl_slice = Some(payload);
            break;
        }
    }
    let stbl = stbl_slice?;

    let mut codec: Option<VideoCodec> = None;
    let mut extradata = Vec::new();
    let mut width = tkhd_width;
    let mut height = tkhd_height;
    let mut stts: Vec<(u32, u32)> = Vec::new();
    let mut ctts: Vec<(u32, i64)> = Vec::new();
    let mut stss: Option<Vec<u32>> = None;
    let mut stsc: Vec<StscEntry> = Vec::new();
    let mut stsz_default: u32 = 0;
    let mut stsz_count: usize = 0;
    let mut stsz_sizes: Vec<u32> = Vec::new();
    let mut chunk_offsets: Vec<u64> = Vec::new();

    for (kind, payload) in BoxIter::new(stbl) {
        let cap = (*remaining_entries).min(MAX_TABLE_ENTRIES);
        match &kind {
            b"stsd" => {
                if let Some((c, w, h, extra)) = parse_stsd(payload) {
                    codec = Some(c);
                    if w > 0 && h > 0 {
                        width = w;
                        height = h;
                    }
                    extradata = extra;
                }
            }
            b"stts" => {
                stts = parse_stts(payload, cap);
                *remaining_entries = remaining_entries.saturating_sub(stts.len());
            }
            b"ctts" => {
                ctts = parse_ctts(payload, cap);
                *remaining_entries = remaining_entries.saturating_sub(ctts.len());
            }
            b"stss" => {
                let s = parse_stss(payload, cap);
                *remaining_entries = remaining_entries.saturating_sub(s.len());
                stss = Some(s);
            }
            b"stsc" => {
                stsc = parse_stsc(payload, cap);
                *remaining_entries = remaining_entries.saturating_sub(stsc.len());
            }
            b"stsz" => {
                if let Some((def, cnt, sizes)) = parse_stsz(payload, cap) {
                    *remaining_entries = remaining_entries.saturating_sub(sizes.len());
                    stsz_default = def;
                    stsz_count = cnt;
                    stsz_sizes = sizes;
                }
            }
            b"stco" => {
                chunk_offsets = parse_stco(payload, cap);
                *remaining_entries = remaining_entries.saturating_sub(chunk_offsets.len());
            }
            b"co64" => {
                chunk_offsets = parse_co64(payload, cap);
                *remaining_entries = remaining_entries.saturating_sub(chunk_offsets.len());
            }
            _ => {}
        }
    }

    let mut codec = codec.unwrap_or_else(|| VideoCodec::Other("unknown".into()));
    // Ignore still-image / cover-art tracks (e.g. 1-sample MJPEG/PNG), matching VLC/FFmpeg 0:V:n.
    if matches!(&codec, VideoCodec::Other(name) if matches!(name.as_str(), "jpeg" | "mjpa" | "mjpb" | "png ")) && stsz_count <= 1 {
        return None;
    }

    // Tracks that switch between multiple sample descriptions (`stsd` entries)
    // require per-chunk codec reconfiguration; decline native preview while
    // keeping the track in `video_tracks` so subsequent track indices stay aligned.
    let multi_desc = stsc.first().is_some_and(|first| {
        stsc.iter().any(|e| e.sample_desc_index != first.sample_desc_index)
    });
    if multi_desc {
        codec = VideoCodec::Other("multi-stsd".into());
    }

    let keyframes = if stsz_count == 0 || chunk_offsets.is_empty() || stsc.is_empty() || multi_desc {
        Vec::new()
    } else {
        build_keyframes(
            timescale,
            edit_offset_ms,
            file_len,
            &stts,
            &ctts,
            stss.as_deref(),
            &stsc,
            stsz_default,
            stsz_count,
            &stsz_sizes,
            &chunk_offsets,
        )
    };

    Some(VideoTrackIndex {
        index: ordinal,
        track_id,
        container: ContainerKind::Mp4,
        codec,
        width,
        height,
        rotation,
        extradata,
        header_strip: Vec::new(),
        keyframes,
    })
}

/// Parses the leading edit-list entries to compute the presentation-time shift
/// in ms: empty edits (`media_time == -1`) add movie-timescale delay, and a
/// positive `media_time` (trimmed start) subtracts media-timescale offset.
fn parse_elst_offset(b: &[u8], movie_timescale: u32, media_timescale: u32) -> i64 {
    if b.len() < 8 || movie_timescale == 0 || media_timescale == 0 {
        return 0;
    }
    let version = b[0];
    let count = be_u32(b, 4).unwrap_or(0);
    if count == 0 {
        return 0;
    }
    let mut off = 8usize;
    let mut empty_delay_ms = 0i128;
    for _ in 0..count.min(4) {
        let (seg_dur, media_time) = if version == 1 {
            let (Some(d), Some(t)) = (be_u64(b, off), be_i64(b, off + 8)) else { break };
            off += 20;
            (d, t)
        } else {
            let (Some(d), Some(t)) = (be_u32(b, off), be_i32(b, off + 4)) else { break };
            off += 12;
            (d as u64, t as i64)
        };
        if media_time == -1 {
            let delta = (seg_dur as i128).saturating_mul(1000) / (movie_timescale as i128);
            empty_delay_ms = empty_delay_ms.saturating_add(delta);
        } else {
            let trim_ms = if media_time > 0 {
                (media_time as i128).saturating_mul(1000) / (media_timescale as i128)
            } else {
                0
            };
            return empty_delay_ms.saturating_sub(trim_ms).clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        }
    }
    empty_delay_ms.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

fn parse_stsd(b: &[u8]) -> Option<(VideoCodec, u32, u32, Vec<u8>)> {
    // FullBox header (4) + entry_count (4)
    let entries = b.get(8..)?;
    let (fourcc, entry) = BoxIter::new(entries).next()?;
    // VisualSampleEntry has 78 bytes of fixed fields before child codec config boxes.
    let width = be_u16(entry, 24).unwrap_or(0) as u32;
    let height = be_u16(entry, 26).unwrap_or(0) as u32;
    let child_boxes = entry.get(78..).unwrap_or(&[]);

    let codec = match &fourcc {
        b"avc1" | b"avc3" => VideoCodec::H264,
        b"hvc1" | b"hev1" => VideoCodec::Hevc,
        b"av01" => VideoCodec::Av1,
        b"vp09" => VideoCodec::Vp9,
        other => VideoCodec::Other(String::from_utf8_lossy(other).into_owned()),
    };

    let want_box: Option<&[u8; 4]> = match codec {
        VideoCodec::H264 => Some(b"avcC"),
        VideoCodec::Hevc => Some(b"hvcC"),
        VideoCodec::Av1 => Some(b"av1C"),
        VideoCodec::Vp9 => Some(b"vpcC"),
        VideoCodec::Other(_) => None,
    };

    let mut extradata = Vec::new();
    if let Some(target) = want_box {
        for (kind, payload) in BoxIter::new(child_boxes) {
            if &kind == target && payload.len() <= 1024 * 1024 {
                extradata = payload.to_vec();
                break;
            }
        }
    }

    Some((codec, width, height, extradata))
}

fn parse_stts(b: &[u8], cap: usize) -> Vec<(u32, u32)> {
    let Some(count) = be_u32(b, 4) else { return Vec::new() };
    let max = (b.len().saturating_sub(8) / 8).min(count as usize).min(cap);
    let mut out = Vec::with_capacity(max);
    for i in 0..max {
        let off = 8 + i * 8;
        if let (Some(c), Some(d)) = (be_u32(b, off), be_u32(b, off + 4)) {
            out.push((c, d));
        }
    }
    out
}

fn parse_ctts(b: &[u8], cap: usize) -> Vec<(u32, i64)> {
    if b.is_empty() {
        return Vec::new();
    }
    let Some(count) = be_u32(b, 4) else { return Vec::new() };
    let max = (b.len().saturating_sub(8) / 8).min(count as usize).min(cap);
    let mut out = Vec::with_capacity(max);
    for i in 0..max {
        let off = 8 + i * 8;
        let Some(c) = be_u32(b, off) else { break };
        // Both v0 (QuickTime negative offsets) and v1 are read as signed i32,
        // matching FFmpeg and VLC.
        let delta = be_i32(b, off + 4).unwrap_or(0) as i64;
        out.push((c, delta));
    }
    out
}

fn parse_stss(b: &[u8], cap: usize) -> Vec<u32> {
    let Some(count) = be_u32(b, 4) else { return Vec::new() };
    let max = (b.len().saturating_sub(8) / 4).min(count as usize).min(cap);
    let mut out = Vec::with_capacity(max);
    for i in 0..max {
        if let Some(n) = be_u32(b, 8 + i * 4)
            && n > 0
        {
            out.push(n);
        }
    }
    out
}

fn parse_stsc(b: &[u8], cap: usize) -> Vec<StscEntry> {
    let Some(count) = be_u32(b, 4) else { return Vec::new() };
    let max = (b.len().saturating_sub(8) / 12).min(count as usize).min(cap);
    let mut out = Vec::with_capacity(max);
    for i in 0..max {
        let off = 8 + i * 12;
        if let (Some(first_chunk), Some(samples_per_chunk), Some(sample_desc_index)) =
            (be_u32(b, off), be_u32(b, off + 4), be_u32(b, off + 8))
            && first_chunk > 0
            && samples_per_chunk > 0
        {
            out.push(StscEntry { first_chunk, samples_per_chunk, sample_desc_index });
        }
    }
    out
}

fn parse_stsz(b: &[u8], cap: usize) -> Option<(u32, usize, Vec<u32>)> {
    let default_size = be_u32(b, 4)?;
    let count = (be_u32(b, 8)? as usize).min(cap);
    if default_size > 0 {
        return Some((default_size, count, Vec::new()));
    }
    let avail = (b.len().saturating_sub(12) / 4).min(count);
    let mut sizes = Vec::with_capacity(avail);
    for i in 0..avail {
        sizes.push(be_u32(b, 12 + i * 4)?);
    }
    Some((0, avail, sizes))
}

fn parse_stco(b: &[u8], cap: usize) -> Vec<u64> {
    let Some(count) = be_u32(b, 4) else { return Vec::new() };
    let max = (b.len().saturating_sub(8) / 4).min(count as usize).min(cap);
    let mut out = Vec::with_capacity(max);
    for i in 0..max {
        if let Some(v) = be_u32(b, 8 + i * 4) {
            out.push(v as u64);
        }
    }
    out
}

fn parse_co64(b: &[u8], cap: usize) -> Vec<u64> {
    let Some(count) = be_u32(b, 4) else { return Vec::new() };
    let max = (b.len().saturating_sub(8) / 8).min(count as usize).min(cap);
    let mut out = Vec::with_capacity(max);
    for i in 0..max {
        if let Some(v) = be_u64(b, 8 + i * 8) {
            out.push(v);
        }
    }
    out
}

/// Computes `(pts_ms, file_offset, size)` for every sync sample in a single
/// linear pass over the sample tables.
#[allow(clippy::too_many_arguments)]
fn build_keyframes(
    timescale: u32,
    edit_offset_ms: i64,
    file_len: u64,
    stts: &[(u32, u32)],
    ctts: &[(u32, i64)],
    stss: Option<&[u32]>,
    stsc: &[StscEntry],
    stsz_default: u32,
    sample_count: usize,
    stsz_sizes: &[u32],
    chunk_offsets: &[u64],
) -> Vec<KeyframeEntry> {
    let sample_size = |idx: usize| -> u32 {
        if stsz_default > 0 { stsz_default } else { stsz_sizes.get(idx).copied().unwrap_or(0) }
    };

    // Build sorted target 0-based sample indices. If `stss` is absent, every
    // sample is a sync sample (intra-only track): stride if there are thousands.
    let mut targets: Vec<usize> = match stss {
        Some(list) => list.iter().filter_map(|&n| n.checked_sub(1).map(|i| i as usize)).filter(|&i| i < sample_count).collect(),
        None => {
            let stride = (sample_count / 2000).max(1);
            (0..sample_count).step_by(stride).collect()
        }
    };
    targets.sort_unstable();
    targets.dedup();
    if targets.is_empty() {
        return Vec::new();
    }

    // Cursors for stts (DTS) and ctts (PTS - DTS).
    let mut stts_i = 0usize;
    let mut stts_rem = stts.first().map_or(0, |e| e.0);
    let mut dts: i64 = 0;

    let mut ctts_i = 0usize;
    let mut ctts_rem = ctts.first().map_or(0, |e| e.0);

    // Cursors for stsc + chunk offsets.
    let mut stsc_i = 0usize;
    let mut chunk_idx = 0usize; // 0-based
    let mut sample_in_chunk = 0u32;
    let mut cur_offset = chunk_offsets[0];

    let mut target_pos = 0usize;
    let mut keyframes = Vec::with_capacity(targets.len());

    for s in 0..sample_count {
        if target_pos >= targets.len() {
            break;
        }
        let sz = sample_size(s);
        let cto = ctts.get(ctts_i).map_or(0, |e| e.1);

        if targets[target_pos] == s {
            let pts_ticks = dts.saturating_add(cto);
            let pts_ms = ((pts_ticks as i128).saturating_mul(1000) / (timescale as i128))
                .saturating_add(edit_offset_ms as i128)
                .clamp(0, i64::MAX as i128) as i64;
            if sz > 0
                && (sz as usize) <= MAX_PACKET_BYTES
                && cur_offset.checked_add(sz as u64).is_some_and(|end| end <= file_len)
            {
                keyframes.push(KeyframeEntry {
                    pts_ms,
                    offset: cur_offset,
                    size: sz,
                    cluster_rel_offset: None,
                    block_number: None,
                });
            }
            target_pos += 1;
        }

        // Advance DTS.
        if let Some(&(_, delta)) = stts.get(stts_i) {
            dts = dts.saturating_add(delta as i64);
            stts_rem = stts_rem.saturating_sub(1);
            while stts_rem == 0 && stts_i + 1 < stts.len() {
                stts_i += 1;
                stts_rem = stts[stts_i].0;
            }
        }
        // Advance CTTS.
        if !ctts.is_empty() && ctts_i < ctts.len() {
            ctts_rem = ctts_rem.saturating_sub(1);
            while ctts_rem == 0 && ctts_i + 1 < ctts.len() {
                ctts_i += 1;
                ctts_rem = ctts[ctts_i].0;
            }
        }
        // Advance chunk offset.
        sample_in_chunk += 1;
        let spc = stsc[stsc_i].samples_per_chunk;
        if sample_in_chunk >= spc {
            chunk_idx += 1;
            if chunk_idx >= chunk_offsets.len() {
                break;
            }
            sample_in_chunk = 0;
            // stsc first_chunk is 1-based.
            while stsc_i + 1 < stsc.len() && (chunk_idx as u32 + 1) >= stsc[stsc_i + 1].first_chunk {
                stsc_i += 1;
            }
            cur_offset = chunk_offsets[chunk_idx];
        } else {
            cur_offset = cur_offset.saturating_add(sz as u64);
        }
    }

    keyframes.sort_by_key(|k| k.pts_ms);
    keyframes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + payload.len());
        out.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn rotation_matrix_decodes_quarter_turns() {
        let matrix = |a: i32, b: i32| {
            let mut buf = vec![0u8; 36];
            buf[0..4].copy_from_slice(&a.to_be_bytes());
            buf[4..8].copy_from_slice(&b.to_be_bytes());
            buf
        };
        assert_eq!(rotation_from_matrix(&matrix(0x10000, 0), 0), 0);
        assert_eq!(rotation_from_matrix(&matrix(0, 0x10000), 0), 90);
        assert_eq!(rotation_from_matrix(&matrix(-0x10000, 0), 0), 180);
        assert_eq!(rotation_from_matrix(&matrix(0, -0x10000), 0), 270);
    }

    #[test]
    fn elst_handles_empty_delay_positive_trim_and_extreme_values() {
        // v0 elst: empty edit of 500 ticks (@1000Hz -> +500ms), then trim at media_time=90000 (@90000Hz -> -1000ms) => -500ms.
        let mut elst = vec![0u8; 8 + 24];
        elst[4..8].copy_from_slice(&2u32.to_be_bytes());
        // Entry 1: seg_dur=500, media_time=-1, rate=1.0 (0x0001_0000)
        elst[8..12].copy_from_slice(&500u32.to_be_bytes());
        elst[12..16].copy_from_slice(&(-1i32).to_be_bytes());
        elst[16..20].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        // Entry 2: seg_dur=5000, media_time=90000, rate=1.0
        elst[20..24].copy_from_slice(&5000u32.to_be_bytes());
        elst[24..28].copy_from_slice(&90_000i32.to_be_bytes());
        elst[28..32].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        assert_eq!(parse_elst_offset(&elst, 1000, 90_000), -500);

        // Crafted huge v1 edit list saturates cleanly without overflow or panic.
        let mut huge = vec![0u8; 8 + 20];
        huge[0] = 1;
        huge[4..8].copy_from_slice(&1u32.to_be_bytes());
        huge[8..16].copy_from_slice(&u64::MAX.to_be_bytes());
        huge[16..24].copy_from_slice(&(-1i64).to_be_bytes());
        assert_eq!(parse_elst_offset(&huge, 1, 1), i64::MAX);
    }

    #[test]
    fn ctts_reads_signed_offsets_for_both_v0_and_v1() {
        for version in [0u8, 1u8] {
            let mut ctts = vec![0u8; 8 + 16];
            ctts[0] = version;
            ctts[4..8].copy_from_slice(&2u32.to_be_bytes());
            // count=1, offset=-1000 (signed)
            ctts[8..12].copy_from_slice(&1u32.to_be_bytes());
            ctts[12..16].copy_from_slice(&(-1000i32).to_be_bytes());
            // count=2, offset=+2000
            ctts[16..20].copy_from_slice(&2u32.to_be_bytes());
            ctts[20..24].copy_from_slice(&2000i32.to_be_bytes());
            assert_eq!(parse_ctts(&ctts, 100), vec![(1, -1000), (2, 2000)]);
        }
    }

    #[test]
    fn quicktime_mdta_keys_and_unrecognized_fallback() {
        // Build `keys`: 4-byte flags + count=2 + key1("com.apple.quicktime.title") + key2("com.apple.quicktime.artist")
        let mut keys_payload = vec![0u8; 8];
        keys_payload[4..8].copy_from_slice(&2u32.to_be_bytes());
        for name in [b"com.apple.quicktime.title".as_slice(), b"com.apple.quicktime.artist".as_slice()] {
            keys_payload.extend_from_slice(&((8 + name.len()) as u32).to_be_bytes());
            keys_payload.extend_from_slice(b"mdta");
            keys_payload.extend_from_slice(name);
        }
        let make_data = |text: &str| {
            let mut p = vec![0u8; 8];
            p[0..4].copy_from_slice(&1u32.to_be_bytes()); // UTF-8
            p.extend_from_slice(text.as_bytes());
            mp4_box(b"data", &p)
        };
        let mut ilst_payload = Vec::new();
        ilst_payload.extend_from_slice(&mp4_box(&1u32.to_be_bytes(), &make_data("QuickTime Film")));
        ilst_payload.extend_from_slice(&mp4_box(&2u32.to_be_bytes(), &make_data("Jane Director")));

        let mut meta_payload = vec![0u8; 4]; // FullBox flags
        meta_payload.extend_from_slice(&mp4_box(b"keys", &keys_payload));
        meta_payload.extend_from_slice(&mp4_box(b"ilst", &ilst_payload));

        let mut meta = MediaMeta::default();
        let mut unparsed = false;
        parse_meta_box(&meta_payload, &mut meta, None, &mut unparsed);
        assert!(!unparsed);
        assert_eq!(meta.title.as_deref(), Some("QuickTime Film"));
        assert_eq!(meta.artist.as_deref(), Some("Jane Director"));

        // Unrecognized mdta key flags `saw_unparsed_mdta = true` so fallback triggers.
        let mut unk_keys = vec![0u8; 8];
        unk_keys[4..8].copy_from_slice(&1u32.to_be_bytes());
        let k = b"com.custom.unknown";
        unk_keys.extend_from_slice(&((8 + k.len()) as u32).to_be_bytes());
        unk_keys.extend_from_slice(b"mdta");
        unk_keys.extend_from_slice(k);
        let unk_ilst = mp4_box(&1u32.to_be_bytes(), &make_data("mystery"));
        let mut unk_meta = vec![0u8; 4];
        unk_meta.extend_from_slice(&mp4_box(b"keys", &unk_keys));
        unk_meta.extend_from_slice(&mp4_box(b"ilst", &unk_ilst));
        let mut meta2 = MediaMeta::default();
        let mut unparsed2 = false;
        parse_meta_box(&unk_meta, &mut meta2, None, &mut unparsed2);
        assert!(unparsed2);

        // End-to-end metadata-only scan on an MP4 with only an unrecognized mdta key
        // returns Err so MetaScanner falls back to libVLC.
        let dir = std::env::temp_dir().join(format!("revlc-mp4-mdta-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("unk-mdta.mp4");
        let ftyp = mp4_box(b"ftyp", b"qt  \x00\x00\x00\x00qt  ");
        let mut mvhd = vec![0u8; 100];
        mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
        mvhd[16..20].copy_from_slice(&5000u32.to_be_bytes());
        let moov = mp4_box(b"moov", &[mp4_box(b"mvhd", &mvhd), mp4_box(b"meta", &unk_meta)].concat());
        std::fs::write(&p, [ftyp, moov].concat()).unwrap();
        assert!(parse_meta(&p, &dir).is_err(), "unknown-only mdta must trigger metadata fallback");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nero_chpl_parses_v1_chapters_and_truncated_child_boxes_stop_cleanly() {
        let mut chpl = vec![1u8, 0, 0, 0, 0, 0, 0, 0, 2]; // v1, 2 chapters
        // Chapter 1 at 0s: "Intro"
        chpl.extend_from_slice(&0u64.to_be_bytes());
        chpl.push(5);
        chpl.extend_from_slice(b"Intro");
        // Chapter 2 at 12.5s (125_000_000 * 100ns): "Act I"
        chpl.extend_from_slice(&125_000_000u64.to_be_bytes());
        chpl.push(5);
        chpl.extend_from_slice(b"Act I");
        let mut chapters = Vec::new();
        parse_chpl(&chpl, &mut chapters);
        assert_eq!(chapters.len(), 2);
        assert_eq!((chapters[0].start, chapters[0].name.as_str()), (0, "Intro"));
        assert_eq!((chapters[1].start, chapters[1].name.as_str()), (12_500, "Act I"));

        // Truncated chpl entry whose name_len runs past the buffer stops cleanly.
        let mut trunc_chpl = vec![1u8, 0, 0, 0, 0, 0, 0, 0, 1];
        trunc_chpl.extend_from_slice(&0u64.to_be_bytes());
        trunc_chpl.push(40);
        trunc_chpl.extend_from_slice(b"abc");
        let mut trunc_out = Vec::new();
        parse_chpl(&trunc_chpl, &mut trunc_out);
        assert!(trunc_out.is_empty());

        // Truncated child box extending past slice end is rejected by BoxIter.
        let truncated = [0, 0, 0, 100, b'f', b'r', b'e', b'e', 1, 2, 3];
        assert_eq!(BoxIter::new(&truncated).count(), 0);
    }

    #[test]
    fn co64_and_multi_stsd_detection() {
        let mut co64 = vec![0u8; 8 + 16];
        co64[4..8].copy_from_slice(&2u32.to_be_bytes());
        co64[8..16].copy_from_slice(&0x1_0000_0010u64.to_be_bytes());
        co64[16..24].copy_from_slice(&0x1_0000_2000u64.to_be_bytes());
        assert_eq!(parse_co64(&co64, 10), vec![0x1_0000_0010, 0x1_0000_2000]);

        // Full video `trak` with `stsc` switching from sample_desc 1 -> 2 must be
        // declined with `VideoCodec::Other("multi-stsd")` and zero keyframes.
        let mut tkhd = vec![0u8; 84];
        tkhd[12..16].copy_from_slice(&1u32.to_be_bytes()); // track_id=1
        tkhd[76..80].copy_from_slice(&(640u32 << 16).to_be_bytes());
        tkhd[80..84].copy_from_slice(&(360u32 << 16).to_be_bytes());
        let mut mdhd = vec![0u8; 24];
        mdhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
        let mut hdlr = vec![0u8; 12];
        hdlr[8..12].copy_from_slice(b"vide");
        let mut avc1_payload = vec![0u8; 78];
        avc1_payload[24..26].copy_from_slice(&640u16.to_be_bytes());
        avc1_payload[26..28].copy_from_slice(&360u16.to_be_bytes());
        let mut stsd = vec![0u8; 8];
        stsd[4..8].copy_from_slice(&1u32.to_be_bytes());
        stsd.extend_from_slice(&mp4_box(b"avc1", &avc1_payload));
        let mut stts = vec![0u8; 16];
        stts[4..8].copy_from_slice(&1u32.to_be_bytes());
        stts[8..12].copy_from_slice(&2u32.to_be_bytes());
        stts[12..16].copy_from_slice(&40u32.to_be_bytes());
        let mut stsc = vec![0u8; 8 + 24];
        stsc[4..8].copy_from_slice(&2u32.to_be_bytes());
        stsc[8..12].copy_from_slice(&1u32.to_be_bytes());
        stsc[12..16].copy_from_slice(&1u32.to_be_bytes());
        stsc[16..20].copy_from_slice(&1u32.to_be_bytes());
        stsc[20..24].copy_from_slice(&2u32.to_be_bytes());
        stsc[24..28].copy_from_slice(&1u32.to_be_bytes());
        stsc[28..32].copy_from_slice(&2u32.to_be_bytes());
        let mut stsz = vec![0u8; 12];
        stsz[4..8].copy_from_slice(&100u32.to_be_bytes()); // default size=100
        stsz[8..12].copy_from_slice(&2u32.to_be_bytes()); // 2 samples
        let mut stco = vec![0u8; 16];
        stco[4..8].copy_from_slice(&2u32.to_be_bytes());
        stco[8..12].copy_from_slice(&1000u32.to_be_bytes());
        stco[12..16].copy_from_slice(&2000u32.to_be_bytes());
        let stbl = mp4_box(
            b"stbl",
            &[
                mp4_box(b"stsd", &stsd),
                mp4_box(b"stts", &stts),
                mp4_box(b"stsc", &stsc),
                mp4_box(b"stsz", &stsz),
                mp4_box(b"stco", &stco),
            ]
            .concat(),
        );
        let minf = mp4_box(b"minf", &stbl);
        let mdia = mp4_box(b"mdia", &[mp4_box(b"mdhd", &mdhd), mp4_box(b"hdlr", &hdlr), minf].concat());
        let trak_payload = [mp4_box(b"tkhd", &tkhd), mdia].concat();
        let mut remaining = 10_000usize;
        let track = parse_video_trak(&trak_payload, 1000, 10_000, 0, &mut remaining).expect("video trak parsed");
        assert_eq!(track.codec, VideoCodec::Other("multi-stsd".into()));
        assert!(track.keyframes.is_empty());
    }
}
