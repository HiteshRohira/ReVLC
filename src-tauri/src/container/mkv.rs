//! Pure-Rust streaming Matroska / WebM (EBML) metadata, chapter, and keyframe
//! (`Cues`) parser + on-demand Cluster/SimpleBlock packet reader.
//!
//! Uses `SeekHead` to jump directly to `Info`, `Tracks`, `Chapters`, `Cues`,
//! `Tags`, and `Attachments` without scanning multi-gigabyte `Cluster` data.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::engine::{Chapter, MediaMeta};

use super::{
    ContainerKind, IndexedMedia, KeyframeEntry, MAX_PACKET_BYTES, VideoCodec, VideoTrackIndex, parse_number_tag,
    write_embedded_art,
};

// Top-level & section EBML element IDs.
const ID_EBML: u32 = 0x1A45_DFA3;
const ID_SEGMENT: u32 = 0x1853_8067;
const ID_SEEK_HEAD: u32 = 0x114D_9B74;
const ID_SEEK: u32 = 0x4DBB;
const ID_SEEK_ID: u32 = 0x53AB;
const ID_SEEK_POSITION: u32 = 0x53AC;
const ID_INFO: u32 = 0x1549_A966;
const ID_TIMESTAMP_SCALE: u32 = 0x002A_D7B1;
const ID_DURATION: u32 = 0x4489;
const ID_TITLE: u32 = 0x7BA9;
const ID_TRACKS: u32 = 0x1654_AE6B;
const ID_TRACK_ENTRY: u32 = 0xAE;
const ID_TRACK_NUMBER: u32 = 0xD7;
const ID_TRACK_TYPE: u32 = 0x83;
const ID_CODEC_ID: u32 = 0x86;
const ID_CODEC_PRIVATE: u32 = 0x63A2;
const ID_VIDEO: u32 = 0xE0;
const ID_PIXEL_WIDTH: u32 = 0xB0;
const ID_PIXEL_HEIGHT: u32 = 0xBA;
const ID_CONTENT_ENCODINGS: u32 = 0x6D80;
const ID_CONTENT_ENCODING: u32 = 0x6240;
const ID_CONTENT_ENCODING_TYPE: u32 = 0x5033;
const ID_CONTENT_COMPRESSION: u32 = 0x5034;
const ID_CONTENT_COMP_ALGO: u32 = 0x4254;
const ID_CONTENT_COMP_SETTINGS: u32 = 0x4255;
const ID_CONTENT_ENCRYPTION: u32 = 0x5035;
const ID_CHAPTERS: u32 = 0x1043_A770;
const ID_EDITION_ENTRY: u32 = 0x45B9;
const ID_CHAPTER_ATOM: u32 = 0xB6;
const ID_CHAPTER_TIME_START: u32 = 0x91;
const ID_CHAPTER_TIME_END: u32 = 0x92;
const ID_CHAPTER_DISPLAY: u32 = 0x80;
const ID_CHAP_STRING: u32 = 0x85;
const ID_CUES: u32 = 0x1C53_BB6B;
const ID_CUE_POINT: u32 = 0xBB;
const ID_CUE_TIME: u32 = 0xB3;
const ID_CUE_TRACK_POSITIONS: u32 = 0xB7;
const ID_CUE_TRACK: u32 = 0xF7;
const ID_CUE_CLUSTER_POSITION: u32 = 0xF1;
const ID_CUE_RELATIVE_POSITION: u32 = 0xF0;
const ID_CUE_BLOCK_NUMBER: u32 = 0x5378;
const ID_TAGS: u32 = 0x1254_C367;
const ID_TAG: u32 = 0x7373;
const ID_SIMPLE_TAG: u32 = 0x67C8;
const ID_TAG_NAME: u32 = 0x45A3;
const ID_TAG_STRING: u32 = 0x4487;
const ID_ATTACHMENTS: u32 = 0x1941_A469;
const ID_ATTACHED_FILE: u32 = 0x61A7;
const ID_FILE_NAME: u32 = 0x466E;
const ID_FILE_MIME_TYPE: u32 = 0x4660;
const ID_FILE_DATA: u32 = 0x465C;
const ID_CLUSTER: u32 = 0x1F43_B675;
const ID_SIMPLE_BLOCK: u32 = 0xA3;
const ID_BLOCK_GROUP: u32 = 0xA0;
const ID_BLOCK: u32 = 0xA1;
const ID_REFERENCE_BLOCK: u32 = 0xFB;

/// Maximum metadata/cues section size we buffer in memory (16 MiB).
const MAX_SECTION_BYTES: u64 = 16 * 1024 * 1024;
/// Maximum Cluster scan window when reading a single keyframe block (16 MiB).
const MAX_CLUSTER_SCAN_BYTES: u64 = 16 * 1024 * 1024;
/// Maximum CuePoint entries indexed per file.
const MAX_CUE_ENTRIES: usize = 500_000;

/// Reads metadata (tags, duration, embedded cover art) from an MKV/WebM file.
pub fn parse_meta(path: &Path, art_dir: &Path) -> Result<MediaMeta, String> {
    let parsed = parse_internal(path, Some(art_dir), false)?;
    Ok(parsed.meta)
}

/// Full index: metadata, chapters, video tracks, and keyframe `Cues` tables.
pub fn index(path: &Path) -> Result<IndexedMedia, String> {
    parse_internal(path, None, true)
}

fn parse_internal(path: &Path, art_dir: Option<&Path>, build_keyframes: bool) -> Result<IndexedMedia, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file_len = file.metadata().map_err(|e| e.to_string())?.len();
    if file_len < 12 {
        return Err("MKV file too small".into());
    }
    let mut reader = BufReader::new(file);

    // 1. Validate EBML header and find Segment start.
    let (ebml_id, ebml_size, ebml_data_pos) = read_element_header(&mut reader, file_len)?;
    if ebml_id != ID_EBML {
        return Err("missing EBML header".into());
    }
    let seg_search_start = ebml_data_pos.checked_add(ebml_size.ok_or("unknown EBML header size")?).ok_or("EBML overflow")?;
    reader.seek(SeekFrom::Start(seg_search_start)).map_err(|e| e.to_string())?;

    let mut seg_data_start = None;
    let mut seg_end = file_len;
    for _ in 0..16 {
        let cur = reader.stream_position().map_err(|e| e.to_string())?;
        if cur >= file_len {
            break;
        }
        let (id, size, data_pos) = read_element_header(&mut reader, file_len)?;
        if id == ID_SEGMENT {
            seg_data_start = Some(data_pos);
            if let Some(sz) = size {
                seg_end = data_pos.saturating_add(sz).min(file_len);
            }
            break;
        }
        let Some(sz) = size else { break };
        reader.seek(SeekFrom::Start(data_pos.saturating_add(sz))).map_err(|e| e.to_string())?;
    }
    let seg_data_start = seg_data_start.ok_or("MKV Segment not found")?;

    // 2. Scan top-level Segment elements before the first Cluster, collecting
    //    SeekHead pointers and any inline Info/Tracks/Chapters/Cues/Tags/Attachments.
    let mut seek_targets: Vec<(u32, u64)> = Vec::new();
    let mut info_pos = None;
    let mut tracks_pos = None;
    let mut chapters_pos = None;
    let mut cues_pos = None;
    let mut tags_pos = None;
    let mut attachments_pos = None;

    let mut pos = seg_data_start;
    for _ in 0..512 {
        if pos >= seg_end {
            break;
        }
        reader.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        let Ok((id, size, data_pos)) = read_element_header(&mut reader, seg_end) else { break };
        if id == ID_CLUSTER {
            break;
        }
        let Some(sz) = size else { break };
        let next = data_pos.saturating_add(sz).min(seg_end);
        match id {
            ID_SEEK_HEAD if sz <= MAX_SECTION_BYTES => {
                if let Ok(buf) = read_bytes_at(&mut reader, data_pos, sz) {
                    parse_seek_head(&buf, seg_data_start, &mut seek_targets);
                }
            }
            ID_INFO => info_pos = Some((data_pos, sz)),
            ID_TRACKS => tracks_pos = Some((data_pos, sz)),
            ID_CHAPTERS => chapters_pos = Some((data_pos, sz)),
            ID_CUES => cues_pos = Some((data_pos, sz)),
            ID_TAGS => tags_pos = Some((data_pos, sz)),
            ID_ATTACHMENTS => attachments_pos = Some((data_pos, sz)),
            _ => {}
        }
        if next <= pos {
            break;
        }
        pos = next;
    }

    // Resolve any remaining sections via SeekHead pointers (follow secondary SeekHead once).
    let mut seen_seekheads = Vec::new();
    let mut idx = 0usize;
    while idx < seek_targets.len() && idx < 64 {
        let (target_id, abs_pos) = seek_targets[idx];
        idx += 1;
        if abs_pos >= seg_end {
            continue;
        }
        let resolve = |r: &mut BufReader<File>, expected: u32| -> Option<(u64, u64)> {
            r.seek(SeekFrom::Start(abs_pos)).ok()?;
            let (id, sz, data_pos) = read_element_header(r, seg_end).ok()?;
            (id == expected).then_some((data_pos, sz?))
        };
        match target_id {
            ID_SEEK_HEAD if !seen_seekheads.contains(&abs_pos) && seen_seekheads.len() < 4 => {
                seen_seekheads.push(abs_pos);
                if let Some((dp, sz)) = resolve(&mut reader, ID_SEEK_HEAD)
                    && sz <= MAX_SECTION_BYTES
                    && let Ok(buf) = read_bytes_at(&mut reader, dp, sz)
                {
                    parse_seek_head(&buf, seg_data_start, &mut seek_targets);
                }
            }
            ID_INFO if info_pos.is_none() => info_pos = resolve(&mut reader, ID_INFO),
            ID_TRACKS if tracks_pos.is_none() && build_keyframes => tracks_pos = resolve(&mut reader, ID_TRACKS),
            ID_CHAPTERS if chapters_pos.is_none() && build_keyframes => chapters_pos = resolve(&mut reader, ID_CHAPTERS),
            ID_CUES if cues_pos.is_none() && build_keyframes => cues_pos = resolve(&mut reader, ID_CUES),
            ID_TAGS if tags_pos.is_none() => tags_pos = resolve(&mut reader, ID_TAGS),
            ID_ATTACHMENTS if attachments_pos.is_none() && art_dir.is_some() => attachments_pos = resolve(&mut reader, ID_ATTACHMENTS),
            _ => {}
        }
    }

    let mut out = IndexedMedia::default();
    let mut timestamp_scale: u64 = 1_000_000; // default 1 ms per tick

    if let Some((dp, sz)) = info_pos
        && sz <= MAX_SECTION_BYTES
        && let Ok(buf) = read_bytes_at(&mut reader, dp, sz)
    {
        parse_info(&buf, &mut timestamp_scale, &mut out.meta);
    }

    if let Some((dp, sz)) = tags_pos
        && sz <= MAX_SECTION_BYTES
        && let Ok(buf) = read_bytes_at(&mut reader, dp, sz)
    {
        parse_tags(&buf, &mut out.meta);
    }

    if let (Some(dir), Some((dp, sz))) = (art_dir, attachments_pos)
        && sz <= MAX_SECTION_BYTES
        && let Ok(buf) = read_bytes_at(&mut reader, dp, sz)
    {
        parse_attachments(&buf, dir, &mut out.meta);
    }

    if build_keyframes {
        if let Some((dp, sz)) = tracks_pos
            && sz <= MAX_SECTION_BYTES
            && let Ok(buf) = read_bytes_at(&mut reader, dp, sz)
        {
            out.video_tracks = parse_tracks(&buf);
        }
        if let Some((dp, sz)) = chapters_pos
            && sz <= MAX_SECTION_BYTES
            && let Ok(buf) = read_bytes_at(&mut reader, dp, sz)
        {
            parse_chapters(&buf, &mut out.chapters);
        }
        if let Some((dp, sz)) = cues_pos
            && sz <= MAX_SECTION_BYTES
            && let Ok(buf) = read_bytes_at(&mut reader, dp, sz)
        {
            parse_cues(&buf, seg_data_start, timestamp_scale, &mut out.video_tracks);
        }
        for track in &mut out.video_tracks {
            track.keyframes.sort_by_key(|k| k.pts_ms);
        }
        for i in 0..out.chapters.len() {
            if out.chapters[i].duration <= 0 {
                let end = out.chapters.get(i + 1).map(|c| c.start).unwrap_or(out.meta.duration);
                out.chapters[i].duration = (end - out.chapters[i].start).max(0);
            }
        }
    }

    Ok(out)
}

/// Reads an EBML element ID (up to 4 bytes) and data size (up to 8 bytes)
/// from `reader`, returning `(id, Option<size>, data_start_offset)`.
fn read_element_header(reader: &mut (impl Read + Seek), limit: u64) -> Result<(u32, Option<u64>, u64), String> {
    let start = reader.stream_position().map_err(|e| e.to_string())?;
    if start >= limit {
        return Err("EOF reading EBML header".into());
    }
    let mut b0 = [0u8; 1];
    reader.read_exact(&mut b0).map_err(|e| e.to_string())?;
    let id_len = b0[0].leading_zeros() as usize + 1;
    if id_len == 0 || id_len > 4 {
        return Err("invalid EBML element ID".into());
    }
    let mut id: u32 = b0[0] as u32;
    if id_len > 1 {
        let mut rest = [0u8; 3];
        reader.read_exact(&mut rest[..id_len - 1]).map_err(|e| e.to_string())?;
        for &byte in &rest[..id_len - 1] {
            id = (id << 8) | byte as u32;
        }
    }

    let mut s0 = [0u8; 1];
    reader.read_exact(&mut s0).map_err(|e| e.to_string())?;
    let sz_len = s0[0].leading_zeros() as usize + 1;
    if sz_len == 0 || sz_len > 8 {
        return Err("invalid EBML element size".into());
    }
    let mask = if sz_len == 8 { 0u8 } else { (1u8 << (8 - sz_len)) - 1 };
    let mut val: u64 = (s0[0] & mask) as u64;
    let mut all_ones = (s0[0] & mask) == mask;
    if sz_len > 1 {
        let mut rest = [0u8; 7];
        reader.read_exact(&mut rest[..sz_len - 1]).map_err(|e| e.to_string())?;
        for &byte in &rest[..sz_len - 1] {
            if byte != 0xFF {
                all_ones = false;
            }
            val = (val << 8) | byte as u64;
        }
    }
    let data_pos = start + id_len as u64 + sz_len as u64;
    Ok((id, (!all_ones).then_some(val), data_pos))
}

fn read_bytes_at(reader: &mut (impl Read + Seek), pos: u64, size: u64) -> Result<Vec<u8>, String> {
    if size > MAX_SECTION_BYTES {
        return Err("EBML section exceeds safety limit".into());
    }
    reader.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; size as usize];
    reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Zero-copy EBML element iterator over a buffered slice.
struct EbmlIter<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> EbmlIter<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
}

impl<'a> Iterator for EbmlIter<'a> {
    type Item = (u32, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let (id, id_len) = read_vint_id(&self.buf[self.pos..])?;
        let (size, sz_len) = read_vint_value(&self.buf[self.pos + id_len..])?;
        let data_start = self.pos + id_len + sz_len;
        let end = match size {
            Some(sz) => data_start.checked_add(usize::try_from(sz).ok()?)?.min(self.buf.len()),
            None => self.buf.len(),
        };
        if end < data_start {
            return None;
        }
        let payload = &self.buf[data_start..end];
        self.pos = end;
        Some((id, payload))
    }
}

fn read_vint_id(b: &[u8]) -> Option<(u32, usize)> {
    let first = *b.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len > 4 || len > b.len() {
        return None;
    }
    let mut id = first as u32;
    for &byte in &b[1..len] {
        id = (id << 8) | byte as u32;
    }
    Some((id, len))
}

fn read_vint_value(b: &[u8]) -> Option<(Option<u64>, usize)> {
    let first = *b.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || len > b.len() {
        return None;
    }
    let mask = if len == 8 { 0u8 } else { (1u8 << (8 - len)) - 1 };
    let mut val = (first & mask) as u64;
    let mut all_ones = (first & mask) == mask;
    for &byte in &b[1..len] {
        if byte != 0xFF {
            all_ones = false;
        }
        val = (val << 8) | byte as u64;
    }
    Some(((!all_ones).then_some(val), len))
}

fn read_uint(b: &[u8]) -> u64 {
    let mut v = 0u64;
    for &byte in b.iter().take(8) {
        v = (v << 8) | byte as u64;
    }
    v
}

fn read_float(b: &[u8]) -> Option<f64> {
    match b.len() {
        4 => Some(f32::from_be_bytes(b.try_into().ok()?) as f64),
        8 => Some(f64::from_be_bytes(b.try_into().ok()?)),
        _ => None,
    }
}

fn read_str(b: &[u8]) -> Option<String> {
    let trimmed = b.split(|&c| c == 0).next().unwrap_or(b);
    let s = String::from_utf8_lossy(trimmed).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn parse_seek_head(b: &[u8], seg_data_start: u64, out: &mut Vec<(u32, u64)>) {
    for (id, payload) in EbmlIter::new(b) {
        if out.len() >= 64 {
            break;
        }
        if id != ID_SEEK {
            continue;
        }
        let mut seek_id: Option<u32> = None;
        let mut seek_pos: Option<u64> = None;
        for (cid, cp) in EbmlIter::new(payload) {
            match cid {
                ID_SEEK_ID if !cp.is_empty() && cp.len() <= 4 => seek_id = Some(read_uint(cp) as u32),
                ID_SEEK_POSITION => seek_pos = Some(read_uint(cp)),
                _ => {}
            }
        }
        // Only retain top-level metadata/index sections so a SeekHead listing
        // thousands of Clusters never crowds out Cues, Tags, or Attachments.
        if let (Some(sid), Some(spos)) = (seek_id, seek_pos)
            && matches!(sid, ID_SEEK_HEAD | ID_INFO | ID_TRACKS | ID_CHAPTERS | ID_CUES | ID_TAGS | ID_ATTACHMENTS)
            && let Some(abs) = seg_data_start.checked_add(spos)
        {
            out.push((sid, abs));
        }
    }
}

fn parse_info(b: &[u8], timestamp_scale: &mut u64, meta: &mut MediaMeta) {
    let mut duration_ticks: Option<f64> = None;
    for (id, payload) in EbmlIter::new(b) {
        match id {
            ID_TIMESTAMP_SCALE => {
                let s = read_uint(payload);
                if s > 0 {
                    *timestamp_scale = s;
                }
            }
            ID_DURATION => duration_ticks = read_float(payload),
            ID_TITLE if meta.title.is_none() => meta.title = read_str(payload),
            _ => {}
        }
    }
    if let Some(dur) = duration_ticks
        && dur.is_finite()
        && dur > 0.0
    {
        let ns = dur * (*timestamp_scale as f64);
        meta.duration = (ns / 1_000_000.0).round().clamp(0.0, i64::MAX as f64) as i64;
    }
}

fn parse_tracks(b: &[u8]) -> Vec<VideoTrackIndex> {
    let mut tracks = Vec::new();
    let mut ordinal = 0usize;
    for (id, payload) in EbmlIter::new(b) {
        if id != ID_TRACK_ENTRY {
            continue;
        }
        let mut track_num: u64 = 0;
        let mut track_type: u64 = 0;
        let mut codec_id = String::new();
        let mut extradata = Vec::new();
        let mut width = 0u32;
        let mut height = 0u32;
        let mut header_strip = Vec::new();
        let mut unsupported_encoding = false;

        for (cid, cp) in EbmlIter::new(payload) {
            match cid {
                ID_TRACK_NUMBER => track_num = read_uint(cp),
                ID_TRACK_TYPE => track_type = read_uint(cp),
                ID_CODEC_ID => codec_id = String::from_utf8_lossy(cp).trim_end_matches('\0').to_string(),
                ID_CODEC_PRIVATE if cp.len() <= 1024 * 1024 => extradata = cp.to_vec(),
                ID_VIDEO => {
                    for (vid, vp) in EbmlIter::new(cp) {
                        match vid {
                            ID_PIXEL_WIDTH => width = read_uint(vp) as u32,
                            ID_PIXEL_HEIGHT => height = read_uint(vp) as u32,
                            _ => {}
                        }
                    }
                }
                ID_CONTENT_ENCODINGS => {
                    for (eid, ep) in EbmlIter::new(cp) {
                        if eid != ID_CONTENT_ENCODING {
                            continue;
                        }
                        let mut enc_type = 0u64; // 0 = compression, 1 = encryption
                        let mut saw_compression = false;
                        for (comp_id, comp_p) in EbmlIter::new(ep) {
                            match comp_id {
                                ID_CONTENT_ENCODING_TYPE => enc_type = read_uint(comp_p),
                                ID_CONTENT_ENCRYPTION => unsupported_encoding = true,
                                ID_CONTENT_COMPRESSION => {
                                    saw_compression = true;
                                    let mut algo = 0u64; // default 0 = zlib
                                    let mut settings = Vec::new();
                                    for (k, v) in EbmlIter::new(comp_p) {
                                        match k {
                                            ID_CONTENT_COMP_ALGO => algo = read_uint(v),
                                            ID_CONTENT_COMP_SETTINGS if v.len() <= 1024 => settings = v.to_vec(),
                                            _ => {}
                                        }
                                    }
                                    // Only algo 3 (Header Stripping) is supported natively;
                                    // zlib/bzlib/lzo must decline native decode.
                                    if algo == 3 {
                                        if !settings.is_empty() {
                                            header_strip = settings;
                                        }
                                    } else {
                                        unsupported_encoding = true;
                                    }
                                }
                                _ => {}
                            }
                        }
                        if enc_type != 0 || !saw_compression {
                            unsupported_encoding = true;
                        }
                    }
                }
                _ => {}
            }
        }
        if track_type != 1 || track_num == 0 {
            continue;
        }
        let codec = if unsupported_encoding {
            VideoCodec::Other(format!("{codec_id}+encoded"))
        } else {
            match codec_id.as_str() {
                "V_MPEG4/ISO/AVC" => VideoCodec::H264,
                "V_MPEGH/ISO/HEVC" => VideoCodec::Hevc,
                "V_AV1" => VideoCodec::Av1,
                "V_VP9" => VideoCodec::Vp9,
                other => VideoCodec::Other(other.to_string()),
            }
        };
        tracks.push(VideoTrackIndex {
            index: ordinal,
            track_id: track_num,
            container: ContainerKind::Mkv,
            codec,
            width,
            height,
            rotation: 0,
            extradata,
            header_strip,
            keyframes: Vec::new(),
        });
        ordinal += 1;
    }
    // VLC registers Matroska tracks from a map ordered by TrackNumber ascending,
    // so sorting by `track_id` keeps our 0-based track index aligned with VLC.
    tracks.sort_by_key(|t| t.track_id);
    for (i, t) in tracks.iter_mut().enumerate() {
        t.index = i;
    }
    tracks
}

fn parse_chapters(b: &[u8], chapters: &mut Vec<Chapter>) {
    for (id, payload) in EbmlIter::new(b) {
        if id != ID_EDITION_ENTRY {
            continue;
        }
        for (aid, ap) in EbmlIter::new(payload) {
            if aid != ID_CHAPTER_ATOM {
                continue;
            }
            let mut start_ns = 0u64;
            let mut end_ns = 0u64;
            let mut name = String::new();
            for (cid, cp) in EbmlIter::new(ap) {
                match cid {
                    ID_CHAPTER_TIME_START => start_ns = read_uint(cp),
                    ID_CHAPTER_TIME_END => end_ns = read_uint(cp),
                    ID_CHAPTER_DISPLAY => {
                        for (did, dp) in EbmlIter::new(cp) {
                            if did == ID_CHAP_STRING && name.is_empty() {
                                name = read_str(dp).unwrap_or_default();
                            }
                        }
                    }
                    _ => {}
                }
            }
            let start = (start_ns / 1_000_000).min(i64::MAX as u64) as i64;
            let end = (end_ns / 1_000_000).min(i64::MAX as u64) as i64;
            let idx = chapters.len() + 1;
            chapters.push(Chapter {
                start,
                duration: (end - start).max(0),
                name: if name.is_empty() { format!("Chapter {idx}") } else { name },
            });
        }
        // First edition only, matching VLC's default chapter list.
        if !chapters.is_empty() {
            break;
        }
    }
}

fn parse_cues(b: &[u8], seg_data_start: u64, timestamp_scale: u64, tracks: &mut [VideoTrackIndex]) {
    let track_map: HashMap<u64, usize> = tracks.iter().enumerate().map(|(i, t)| (t.track_id, i)).collect();
    if track_map.is_empty() {
        return;
    }
    let mut total_cues = 0usize;
    for (id, payload) in EbmlIter::new(b) {
        if total_cues >= MAX_CUE_ENTRIES {
            break;
        }
        if id != ID_CUE_POINT {
            continue;
        }
        let mut cue_time: Option<u64> = None;
        let mut positions: Vec<(u64, u64, Option<u64>, Option<u64>)> = Vec::new();
        for (cid, cp) in EbmlIter::new(payload) {
            match cid {
                ID_CUE_TIME => cue_time = Some(read_uint(cp)),
                ID_CUE_TRACK_POSITIONS => {
                    let mut track = 0u64;
                    let mut cluster_pos: Option<u64> = None;
                    let mut rel_pos: Option<u64> = None;
                    let mut block_num: Option<u64> = None;
                    for (pid, pp) in EbmlIter::new(cp) {
                        match pid {
                            ID_CUE_TRACK => track = read_uint(pp),
                            ID_CUE_CLUSTER_POSITION => cluster_pos = Some(read_uint(pp)),
                            ID_CUE_RELATIVE_POSITION => rel_pos = Some(read_uint(pp)),
                            ID_CUE_BLOCK_NUMBER => block_num = Some(read_uint(pp)),
                            _ => {}
                        }
                    }
                    if track > 0
                        && let Some(cpos) = cluster_pos
                    {
                        positions.push((track, cpos, rel_pos, block_num));
                    }
                }
                _ => {}
            }
        }
        let Some(ticks) = cue_time else { continue };
        let pts_ms = ((ticks as u128).saturating_mul(timestamp_scale as u128) / 1_000_000).min(i64::MAX as u128) as i64;
        for (track_num, cpos, rel_pos, block_num) in positions {
            let Some(abs_cluster) = seg_data_start.checked_add(cpos) else { continue };
            if let Some(&t_idx) = track_map.get(&track_num) {
                tracks[t_idx].keyframes.push(KeyframeEntry {
                    pts_ms,
                    offset: abs_cluster,
                    size: 0,
                    cluster_rel_offset: rel_pos,
                    block_number: block_num,
                });
                total_cues += 1;
            }
        }
    }
}

fn parse_tags(b: &[u8], meta: &mut MediaMeta) {
    for (id, payload) in EbmlIter::new(b) {
        if id != ID_TAG {
            continue;
        }
        for (sid, sp) in EbmlIter::new(payload) {
            if sid != ID_SIMPLE_TAG {
                continue;
            }
            let mut name = String::new();
            let mut value = None;
            for (k, v) in EbmlIter::new(sp) {
                match k {
                    ID_TAG_NAME => name = read_str(v).unwrap_or_default().to_ascii_uppercase(),
                    ID_TAG_STRING => value = read_str(v),
                    _ => {}
                }
            }
            let Some(val) = value else { continue };
            match name.as_str() {
                "TITLE" if meta.title.is_none() => meta.title = Some(val),
                "ARTIST" if meta.artist.is_none() => meta.artist = Some(val),
                "ALBUM" if meta.album.is_none() => meta.album = Some(val),
                "ALBUM_ARTIST" | "ALBUMARTIST" if meta.album_artist.is_none() => meta.album_artist = Some(val),
                "GENRE" if meta.genre.is_none() => meta.genre = Some(val),
                "DATE_RELEASED" | "DATE" if meta.date.is_none() => meta.date = Some(val),
                "PART_NUMBER" | "TRACKNUMBER" if meta.track.is_none() => meta.track = parse_number_tag(&val),
                "DISC_NUMBER" | "DISCNUMBER" if meta.disc.is_none() => meta.disc = parse_number_tag(&val),
                _ => {}
            }
        }
    }
}

fn parse_attachments(b: &[u8], art_dir: &Path, meta: &mut MediaMeta) {
    if meta.artwork.is_some() {
        return;
    }
    let mut best_cover: Option<&[u8]> = None;
    let mut first_image: Option<&[u8]> = None;
    for (id, payload) in EbmlIter::new(b) {
        if id != ID_ATTACHED_FILE {
            continue;
        }
        let mut fname = String::new();
        let mut mime = String::new();
        let mut data: Option<&[u8]> = None;
        for (aid, ap) in EbmlIter::new(payload) {
            match aid {
                ID_FILE_NAME => fname = read_str(ap).unwrap_or_default().to_ascii_lowercase(),
                ID_FILE_MIME_TYPE => mime = read_str(ap).unwrap_or_default().to_ascii_lowercase(),
                ID_FILE_DATA => data = Some(ap),
                _ => {}
            }
        }
        let Some(bytes) = data else { continue };
        let is_img = mime.starts_with("image/") || fname.ends_with(".jpg") || fname.ends_with(".jpeg") || fname.ends_with(".png");
        if !is_img {
            continue;
        }
        if fname.contains("cover") || fname.contains("poster") || fname.contains("folder") {
            best_cover = Some(bytes);
            break;
        }
        if first_image.is_none() {
            first_image = Some(bytes);
        }
    }
    if let Some(bytes) = best_cover.or(first_image) {
        meta.artwork = write_embedded_art(art_dir, bytes);
    }
}

/// Seeks to the MKV `Cluster` referenced by `kf` and extracts the raw video
/// frame payload for `track`, prepending any header-stripping bytes.
pub fn read_keyframe_packet(
    file: &mut File,
    file_len: u64,
    track: &VideoTrackIndex,
    kf: &KeyframeEntry,
) -> Result<Vec<u8>, String> {
    if kf.offset >= file_len {
        return Err("MKV cue cluster offset past EOF".into());
    }
    file.seek(SeekFrom::Start(kf.offset)).map_err(|e| e.to_string())?;
    let (id, size, cluster_data_start) = read_element_header(file, file_len)?;
    if id != ID_CLUSTER {
        return Err(format!("expected MKV Cluster at offset {}, got 0x{id:x}", kf.offset));
    }
    let cluster_end = match size {
        Some(sz) => cluster_data_start.saturating_add(sz).min(file_len),
        None => file_len,
    };

    // Fast path: CueRelativePosition points directly to the SimpleBlock / BlockGroup inside Cluster.
    if let Some(rel) = kf.cluster_rel_offset
        && let Some(block_pos) = cluster_data_start.checked_add(rel)
        && block_pos < cluster_end
    {
        file.seek(SeekFrom::Start(block_pos)).map_err(|e| e.to_string())?;
        if let Ok((bid, Some(bsz), bdata)) = read_element_header(file, cluster_end)
            && let Some(pkt) = extract_block_if_matching(file, cluster_end, bid, bdata, bsz, track, true)
        {
            return Ok(pkt);
        }
    }

    // Standard path: scan elements inside this single Cluster until we hit the
    // matching video track's keyframe (honoring RFC 9559 cluster-wide CueBlockNumber).
    let scan_limit = cluster_end.min(cluster_data_start.saturating_add(MAX_CLUSTER_SCAN_BYTES));
    let mut pos = cluster_data_start;
    let mut cluster_blocks_seen = 0u64;
    let mut track_blocks_seen = 0u64;
    let mut first_keyframe: Option<Vec<u8>> = None;

    for _ in 0..4096 {
        if pos >= scan_limit {
            break;
        }
        file.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        let Ok((eid, Some(esz), edata)) = read_element_header(file, scan_limit) else { break };
        let next = edata.saturating_add(esz).min(scan_limit);
        if eid == ID_SIMPLE_BLOCK || eid == ID_BLOCK_GROUP {
            cluster_blocks_seen += 1;
            if let Some((is_track, is_kf, pkt)) = probe_block(file, scan_limit, eid, edata, esz, track)
                && is_track
            {
                track_blocks_seen += 1;
                if is_kf {
                    if let Some(want_num) = kf.block_number {
                        if cluster_blocks_seen == want_num || track_blocks_seen == want_num {
                            return Ok(pkt);
                        }
                    } else {
                        return Ok(pkt);
                    }
                    if first_keyframe.is_none() {
                        first_keyframe = Some(pkt);
                    }
                }
            }
        }
        if next <= pos {
            break;
        }
        pos = next;
    }

    first_keyframe.ok_or_else(|| "no matching video keyframe block in MKV cluster".into())
}

fn extract_block_if_matching(
    file: &mut File,
    limit: u64,
    eid: u32,
    edata: u64,
    esz: u64,
    track: &VideoTrackIndex,
    require_keyframe: bool,
) -> Option<Vec<u8>> {
    let (is_track, is_kf, pkt) = probe_block(file, limit, eid, edata, esz, track)?;
    (is_track && (!require_keyframe || is_kf)).then_some(pkt)
}

/// Parses a `SimpleBlock` or `BlockGroup` -> `Block`. Returns `(matches_track, is_keyframe, frame_bytes)`.
fn probe_block(
    file: &mut File,
    limit: u64,
    eid: u32,
    edata: u64,
    esz: u64,
    track: &VideoTrackIndex,
) -> Option<(bool, bool, Vec<u8>)> {
    if esz == 0 || esz as usize > MAX_PACKET_BYTES || edata.saturating_add(esz) > limit {
        return None;
    }
    if eid == ID_SIMPLE_BLOCK {
        // Peek track VINT first so we don't allocate/read audio blocks.
        file.seek(SeekFrom::Start(edata)).ok()?;
        let mut head = [0u8; 16];
        let n = file.read(&mut head).ok()?;
        let (tnum_opt, tlen) = read_vint_value(&head[..n])?;
        let tnum = tnum_opt?;
        if tnum != track.track_id {
            return Some((false, false, Vec::new()));
        }
        if n < tlen + 3 {
            return None;
        }
        let flags = head[tlen + 2];
        let is_kf = (flags & 0x80) != 0;
        let lacing = (flags >> 1) & 0x03;
        let payload_start = edata + (tlen + 3) as u64;
        let payload_len = (edata + esz).saturating_sub(payload_start) as usize;
        let raw = read_exact_bounded(file, payload_start, payload_len)?;
        let frame = extract_first_laced_frame(&raw, lacing)?;
        return Some((true, is_kf, prepend_header_strip(&track.header_strip, frame)));
    }
    if eid == ID_BLOCK_GROUP {
        let group_buf = read_exact_bounded(file, edata, esz as usize)?;
        let mut has_reference = false;
        let mut block_payload: Option<&[u8]> = None;
        for (cid, cp) in EbmlIter::new(&group_buf) {
            match cid {
                ID_REFERENCE_BLOCK => has_reference = true,
                ID_BLOCK => block_payload = Some(cp),
                _ => {}
            }
        }
        let cp = block_payload?;
        let (tnum_opt, tlen) = read_vint_value(cp)?;
        if tnum_opt? != track.track_id {
            return Some((false, false, Vec::new()));
        }
        if cp.len() < tlen + 3 {
            return None;
        }
        let flags = cp[tlen + 2];
        let lacing = (flags >> 1) & 0x03;
        let frame = extract_first_laced_frame(&cp[tlen + 3..], lacing)?;
        // Per Matroska spec, a BlockGroup is a keyframe iff it has no ReferenceBlock.
        return Some((true, !has_reference, prepend_header_strip(&track.header_strip, frame)));
    }
    None
}

fn read_exact_bounded(file: &mut File, pos: u64, len: usize) -> Option<Vec<u8>> {
    if len == 0 || len > MAX_PACKET_BYTES {
        return None;
    }
    file.seek(SeekFrom::Start(pos)).ok()?;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn prepend_header_strip(prefix: &[u8], frame: &[u8]) -> Vec<u8> {
    if prefix.is_empty() {
        return frame.to_vec();
    }
    let mut out = Vec::with_capacity(prefix.len() + frame.len());
    out.extend_from_slice(prefix);
    out.extend_from_slice(frame);
    out
}

/// Extracts the first frame from a Matroska block payload (handles no lacing,
/// Xiph lacing, fixed-size lacing, and EBML lacing).
fn extract_first_laced_frame(payload: &[u8], lacing: u8) -> Option<&[u8]> {
    match lacing {
        0 => (!payload.is_empty()).then_some(payload),
        // Xiph lacing
        1 => {
            let num_frames = (*payload.first()? as usize) + 1;
            let mut off = 1usize;
            let mut first_size = 0usize;
            for i in 0..num_frames.saturating_sub(1) {
                let mut sz = 0usize;
                loop {
                    let b = *payload.get(off)? as usize;
                    off += 1;
                    sz = sz.checked_add(b)?;
                    if b < 255 {
                        break;
                    }
                }
                if i == 0 {
                    first_size = sz;
                }
            }
            payload.get(off..off.checked_add(first_size)?)
        }
        // Fixed-size lacing
        2 => {
            let num_frames = (*payload.first()? as usize) + 1;
            let data = payload.get(1..)?;
            let frame_size = data.len() / num_frames;
            data.get(..frame_size)
        }
        // EBML lacing
        3 => {
            let num_frames = (*payload.first()? as usize) + 1;
            let mut off = 1usize;
            let (first_opt, len) = read_vint_value(payload.get(off..)?)?;
            let first_size = usize::try_from(first_opt?).ok()?;
            off += len;
            for _ in 1..num_frames.saturating_sub(1) {
                let (_, l) = read_vint_value(payload.get(off..)?)?;
                off += l;
            }
            payload.get(off..off.checked_add(first_size)?)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ebml_id_bytes(id: u32) -> Vec<u8> {
        let b = id.to_be_bytes();
        let first = b.iter().position(|&x| x != 0).unwrap_or(3);
        b[first..].to_vec()
    }

    fn ebml_vint_size(len: usize) -> Vec<u8> {
        if len < 0x7F {
            vec![0x80 | (len as u8)]
        } else {
            let v = len as u16;
            vec![0x40 | ((v >> 8) as u8), (v & 0xFF) as u8]
        }
    }

    fn ebml_elem(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = ebml_id_bytes(id);
        out.extend_from_slice(&ebml_vint_size(payload.len()));
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn vint_decoding_handles_lengths_and_unknown_size() {
        assert_eq!(read_vint_value(&[0x85]), Some((Some(5), 1)));
        assert_eq!(read_vint_value(&[0xFF]), Some((None, 1)));
        assert_eq!(read_vint_value(&[0x40, 0x02]), Some((Some(2), 2)));
        assert_eq!(read_vint_id(&[0x1A, 0x45, 0xDF, 0xA3]), Some((ID_EBML, 4)));
    }

    #[test]
    fn parse_tracks_sorts_by_track_number_and_guards_content_encodings() {
        // Track 2 (HEVC, header-stripped) placed BEFORE Track 1 (H.264, zlib-compressed)
        let enc_strip = ebml_elem(
            ID_CONTENT_ENCODINGS,
            &ebml_elem(
                ID_CONTENT_ENCODING,
                &ebml_elem(
                    ID_CONTENT_COMPRESSION,
                    &[
                        ebml_elem(ID_CONTENT_COMP_ALGO, &[3]),
                        ebml_elem(ID_CONTENT_COMP_SETTINGS, &[0x00, 0x00, 0x01]),
                    ]
                    .concat(),
                ),
            ),
        );
        let enc_zlib = ebml_elem(
            ID_CONTENT_ENCODINGS,
            &ebml_elem(
                ID_CONTENT_ENCODING,
                &ebml_elem(ID_CONTENT_COMPRESSION, &ebml_elem(ID_CONTENT_COMP_ALGO, &[0])),
            ),
        );

        let track2 = ebml_elem(
            ID_TRACK_ENTRY,
            &[
                ebml_elem(ID_TRACK_NUMBER, &[2]),
                ebml_elem(ID_TRACK_TYPE, &[1]),
                ebml_elem(ID_CODEC_ID, b"V_MPEGH/ISO/HEVC"),
                enc_strip,
            ]
            .concat(),
        );
        let track1 = ebml_elem(
            ID_TRACK_ENTRY,
            &[
                ebml_elem(ID_TRACK_NUMBER, &[1]),
                ebml_elem(ID_TRACK_TYPE, &[1]),
                ebml_elem(ID_CODEC_ID, b"V_MPEG4/ISO/AVC"),
                enc_zlib,
            ]
            .concat(),
        );

        let tracks = parse_tracks(&[track2, track1].concat());
        assert_eq!(tracks.len(), 2);
        // Sorted ascending by TrackNumber to match libVLC.
        assert_eq!(tracks[0].track_id, 1);
        assert_eq!(tracks[0].index, 0);
        // zlib-compressed track is declined for native HW decode.
        assert!(!tracks[0].codec.is_hw_preview_supported());
        assert_eq!(tracks[1].track_id, 2);
        assert_eq!(tracks[1].index, 1);
        assert_eq!(tracks[1].codec, VideoCodec::Hevc);
        assert_eq!(tracks[1].header_strip, vec![0x00, 0x00, 0x01]);
    }

    #[test]
    fn seek_head_filters_out_clusters_so_cues_are_never_crowded_out() {
        let mut buf = Vec::new();
        // 80 Cluster seek entries followed by 1 Cues entry.
        for i in 0u64..80 {
            let seek = ebml_elem(
                ID_SEEK,
                &[
                    ebml_elem(ID_SEEK_ID, &ebml_id_bytes(ID_CLUSTER)),
                    ebml_elem(ID_SEEK_POSITION, &[(i + 1) as u8]),
                ]
                .concat(),
            );
            buf.extend_from_slice(&seek);
        }
        buf.extend_from_slice(&ebml_elem(
            ID_SEEK,
            &[
                ebml_elem(ID_SEEK_ID, &ebml_id_bytes(ID_CUES)),
                ebml_elem(ID_SEEK_POSITION, &[0x20, 0x00]),
            ]
            .concat(),
        ));
        let mut out = Vec::new();
        parse_seek_head(&buf, 100, &mut out);
        assert_eq!(out, vec![(ID_CUES, 100 + 0x2000)]);
    }

    #[test]
    fn block_group_with_reference_block_is_not_classified_as_keyframe() {
        let dir = std::env::temp_dir().join(format!("revlc-mkv-bg-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("bg.bin");

        // Block payload: track=1 (0x81), timecode=0x0000, flags=0x00, frame=[0xAA, 0xBB]
        let blk = ebml_elem(ID_BLOCK, &[0x81, 0x00, 0x00, 0x00, 0xAA, 0xBB]);
        let ref_blk = ebml_elem(ID_REFERENCE_BLOCK, &[0xFE]);
        let bg_delta = [blk.clone(), ref_blk].concat();
        let bg_key = blk;

        let mut all = Vec::new();
        let delta_off = 0u64;
        all.extend_from_slice(&bg_delta);
        let key_off = all.len() as u64;
        all.extend_from_slice(&bg_key);
        std::fs::write(&p, &all).unwrap();

        let track = VideoTrackIndex {
            index: 0,
            track_id: 1,
            container: ContainerKind::Mkv,
            codec: VideoCodec::H264,
            width: 320,
            height: 180,
            rotation: 0,
            extradata: Vec::new(),
            header_strip: Vec::new(),
            keyframes: Vec::new(),
        };
        let mut f = File::open(&p).unwrap();
        let (is_t1, is_kf1, _) = probe_block(&mut f, all.len() as u64, ID_BLOCK_GROUP, delta_off, bg_delta.len() as u64, &track).unwrap();
        assert!(is_t1 && !is_kf1, "BlockGroup with ReferenceBlock must not be a keyframe");

        let (is_t2, is_kf2, pkt) = probe_block(&mut f, all.len() as u64, ID_BLOCK_GROUP, key_off, bg_key.len() as u64, &track).unwrap();
        assert!(is_t2 && is_kf2, "BlockGroup without ReferenceBlock must be a keyframe");
        assert_eq!(pkt, vec![0xAA, 0xBB]);

        // End-to-end `read_keyframe_packet` over a Cluster skips the earlier delta BlockGroup
        // and prepends `header_strip` bytes onto the keyframe BlockGroup's payload.
        let cluster = ebml_elem(
            ID_CLUSTER,
            &[
                ebml_elem(ID_BLOCK_GROUP, &bg_delta),
                ebml_elem(ID_BLOCK_GROUP, &bg_key),
            ]
            .concat(),
        );
        let cp = dir.join("cluster.bin");
        std::fs::write(&cp, &cluster).unwrap();
        let stripped_track = VideoTrackIndex { header_strip: vec![0x00, 0x00, 0x00, 0x01], ..track };
        let kf = KeyframeEntry { pts_ms: 0, offset: 0, size: 0, cluster_rel_offset: None, block_number: None };
        let mut cf = File::open(&cp).unwrap();
        let extracted = read_keyframe_packet(&mut cf, cluster.len() as u64, &stripped_track, &kf).unwrap();
        assert_eq!(extracted, vec![0x00, 0x00, 0x00, 0x01, 0xAA, 0xBB]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_tags_and_chapters_extract_expected_fields() {
        let tags = ebml_elem(
            ID_TAG,
            &[
                ebml_elem(ID_SIMPLE_TAG, &[ebml_elem(ID_TAG_NAME, b"TITLE"), ebml_elem(ID_TAG_STRING, b"Sintel")].concat()),
                ebml_elem(ID_SIMPLE_TAG, &[ebml_elem(ID_TAG_NAME, b"ARTIST"), ebml_elem(ID_TAG_STRING, b"Blender")].concat()),
                ebml_elem(ID_SIMPLE_TAG, &[ebml_elem(ID_TAG_NAME, b"TRACKNUMBER"), ebml_elem(ID_TAG_STRING, b"3/12")].concat()),
            ]
            .concat(),
        );
        let mut meta = MediaMeta::default();
        parse_tags(&tags, &mut meta);
        assert_eq!(meta.title.as_deref(), Some("Sintel"));
        assert_eq!(meta.artist.as_deref(), Some("Blender"));
        assert_eq!(meta.track, Some(3));

        let chaps = ebml_elem(
            ID_EDITION_ENTRY,
            &ebml_elem(
                ID_CHAPTER_ATOM,
                &[
                    ebml_elem(ID_CHAPTER_TIME_START, &1_000_000_000u64.to_be_bytes()),
                    ebml_elem(ID_CHAPTER_TIME_END, &5_000_000_000u64.to_be_bytes()),
                    ebml_elem(ID_CHAPTER_DISPLAY, &ebml_elem(ID_CHAP_STRING, b"Opening")),
                ]
                .concat(),
            ),
        );
        let mut out = Vec::new();
        parse_chapters(&chaps, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].start, out[0].duration, out[0].name.as_str()), (1000, 4000, "Opening"));
    }
}
