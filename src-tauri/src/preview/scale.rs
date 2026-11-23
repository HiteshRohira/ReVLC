//! In-memory pixel scaling, rotation, JPEG encoding, and H.264/HEVC framing
//! helpers for native hardware preview decoders.

use image::ExtendedColorType;
use image::codecs::jpeg::JpegEncoder;

/// Downscales a 32-bit BGRA frame (e.g. from VideoToolbox or Media Foundation
/// RGB32) with optional display rotation (0/90/180/270 clockwise) and optional
/// bottom-up row order (`bottom_up = true` for negative-stride Windows RGB32)
/// into a JPEG of width `target_width` (preserving aspect ratio).
#[cfg(any(target_os = "macos", test))]
pub fn bgra_to_scaled_jpeg(
    bgra: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    rotation: u16,
    target_width: u32,
) -> Result<Vec<u8>, String> {
    bgra_to_scaled_jpeg_oriented(bgra, width, height, stride, false, rotation, target_width)
}

pub fn bgra_to_scaled_jpeg_oriented(
    bgra: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    bottom_up: bool,
    rotation: u16,
    target_width: u32,
) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 || target_width == 0 {
        return Err("empty frame dimensions".into());
    }
    let min_stride = (width as usize).checked_mul(4).ok_or("stride overflow")?;
    if stride < min_stride {
        return Err("BGRA stride smaller than width*4".into());
    }
    let needed = stride.checked_mul(height as usize).ok_or("BGRA buffer overflow")?;
    if bgra.len() < needed {
        return Err("truncated BGRA buffer".into());
    }

    let quarter = matches!(rotation % 360, 90 | 270);
    let (rot_w, rot_h) = if quarter { (height, width) } else { (width, height) };
    let out_w = target_width.min(rot_w).max(2) & !1;
    let out_h = ((rot_h as u64 * out_w as u64) / rot_w as u64).clamp(2, out_w as u64 * 4) as u32 & !1;

    let mut rgb = vec![0u8; (out_w as usize) * (out_h as usize) * 3];
    let rot = rotation % 360;

    // Box/area downsampling for crisp, alias-free thumbnails.
    let step_x = (rot_w as f64) / (out_w as f64);
    let step_y = (rot_h as f64) / (out_h as f64);
    let tap_x = (step_x.floor() as u32).clamp(1, 4);
    let tap_y = (step_y.floor() as u32).clamp(1, 4);

    for oy in 0..out_h {
        let ry0 = ((oy as f64) * step_y) as u32;
        for ox in 0..out_w {
            let rx0 = ((ox as f64) * step_x) as u32;
            let (mut r_sum, mut g_sum, mut b_sum, mut cnt) = (0u32, 0u32, 0u32, 0u32);
            for dy in 0..tap_y {
                let ry = (ry0 + dy).min(rot_h - 1);
                for dx in 0..tap_x {
                    let rx = (rx0 + dx).min(rot_w - 1);
                    let (sx, sy_top) = unrotate(rx, ry, width, height, rot);
                    let sy = if bottom_up { height - 1 - sy_top } else { sy_top };
                    let off = (sy as usize) * stride + (sx as usize) * 4;
                    b_sum += bgra[off] as u32;
                    g_sum += bgra[off + 1] as u32;
                    r_sum += bgra[off + 2] as u32;
                    cnt += 1;
                }
            }
            let dst = ((oy as usize) * (out_w as usize) + (ox as usize)) * 3;
            rgb[dst] = (r_sum / cnt) as u8;
            rgb[dst + 1] = (g_sum / cnt) as u8;
            rgb[dst + 2] = (b_sum / cnt) as u8;
        }
    }

    encode_rgb_jpeg(&rgb, out_w, out_h)
}

#[inline]
#[cfg(any(windows, test))]
fn yuv_to_rgb(y_val: i32, u_val: i32, v_val: i32, bt709: bool) -> (u8, u8, u8) {
    let c = (y_val - 16).max(0) * 298;
    let (r, g, b) = if bt709 {
        // BT.709 limited-range integer matrix
        (
            (c + 459 * v_val + 128) >> 8,
            (c - 55 * u_val - 136 * v_val + 128) >> 8,
            (c + 541 * u_val + 128) >> 8,
        )
    } else {
        // BT.601 limited-range integer matrix
        (
            (c + 409 * v_val + 128) >> 8,
            (c - 100 * u_val - 208 * v_val + 128) >> 8,
            (c + 516 * u_val + 128) >> 8,
        )
    };
    (r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8)
}

/// Downscales an NV12 (8-bit Y plane + interleaved UV plane) frame to
/// `target_width` JPEG bytes in memory. Used by Windows Media Foundation decoders.
#[cfg(any(windows, test))]
#[allow(clippy::too_many_arguments)]
pub fn nv12_to_scaled_jpeg(
    y_plane: &[u8],
    uv_plane: &[u8],
    width: u32,
    height: u32,
    y_stride: usize,
    uv_stride: usize,
    rotation: u16,
    target_width: u32,
) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 || target_width == 0 {
        return Err("empty NV12 frame".into());
    }
    if y_stride < width as usize || uv_stride < (width as usize & !1) {
        return Err("invalid NV12 stride".into());
    }
    let y_need = y_stride.checked_mul(height as usize).ok_or("Y plane overflow")?;
    let uv_h = (height as usize).div_ceil(2);
    let uv_need = uv_stride.checked_mul(uv_h).ok_or("UV plane overflow")?;
    if y_plane.len() < y_need || uv_plane.len() < uv_need {
        return Err("truncated NV12 planes".into());
    }

    let bt709 = width >= 1280 || height >= 720;
    let quarter = matches!(rotation % 360, 90 | 270);
    let (rot_w, rot_h) = if quarter { (height, width) } else { (width, height) };
    let out_w = target_width.min(rot_w).max(2) & !1;
    let out_h = ((rot_h as u64 * out_w as u64) / rot_w as u64).clamp(2, out_w as u64 * 4) as u32 & !1;
    let rot = rotation % 360;

    let mut rgb = vec![0u8; (out_w as usize) * (out_h as usize) * 3];
    for oy in 0..out_h {
        let ry = ((oy as u64 * rot_h as u64) / out_h as u64).min(rot_h as u64 - 1) as u32;
        for ox in 0..out_w {
            let rx = ((ox as u64 * rot_w as u64) / out_w as u64).min(rot_w as u64 - 1) as u32;
            let (sx, sy) = unrotate(rx, ry, width, height, rot);
            let y_val = y_plane[sy as usize * y_stride + sx as usize] as i32;
            let uv_off = (sy as usize / 2) * uv_stride + (sx as usize & !1);
            let u_val = uv_plane.get(uv_off).copied().unwrap_or(128) as i32 - 128;
            let v_val = uv_plane.get(uv_off + 1).copied().unwrap_or(128) as i32 - 128;
            let (r, g, b) = yuv_to_rgb(y_val, u_val, v_val, bt709);
            let dst = ((oy as usize) * (out_w as usize) + (ox as usize)) * 3;
            rgb[dst] = r;
            rgb[dst + 1] = g;
            rgb[dst + 2] = b;
        }
    }

    encode_rgb_jpeg(&rgb, out_w, out_h)
}

/// Downscales a P010 (10-bit MSB-aligned in little-endian `u16` Y plane +
/// interleaved UV plane) frame to `target_width` JPEG bytes. Used by Windows
/// Media Foundation Main10 HEVC / 10-bit AV1 decoders.
#[cfg(any(windows, test))]
#[allow(clippy::too_many_arguments)]
pub fn p010_to_scaled_jpeg(
    y_plane: &[u8],
    uv_plane: &[u8],
    width: u32,
    height: u32,
    y_stride: usize,
    uv_stride: usize,
    rotation: u16,
    target_width: u32,
) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 || target_width == 0 {
        return Err("empty P010 frame".into());
    }
    let min_y_stride = (width as usize).checked_mul(2).ok_or("P010 stride overflow")?;
    let min_uv_stride = (width as usize & !1).checked_mul(2).ok_or("P010 UV stride overflow")?;
    if y_stride < min_y_stride || uv_stride < min_uv_stride {
        return Err("invalid P010 stride".into());
    }
    let y_need = y_stride.checked_mul(height as usize).ok_or("P010 Y overflow")?;
    let uv_h = (height as usize).div_ceil(2);
    let uv_need = uv_stride.checked_mul(uv_h).ok_or("P010 UV overflow")?;
    if y_plane.len() < y_need || uv_plane.len() < uv_need {
        return Err("truncated P010 planes".into());
    }

    let quarter = matches!(rotation % 360, 90 | 270);
    let (rot_w, rot_h) = if quarter { (height, width) } else { (width, height) };
    let out_w = target_width.min(rot_w).max(2) & !1;
    let out_h = ((rot_h as u64 * out_w as u64) / rot_w as u64).clamp(2, out_w as u64 * 4) as u32 & !1;
    let rot = rotation % 360;

    let mut rgb = vec![0u8; (out_w as usize) * (out_h as usize) * 3];
    for oy in 0..out_h {
        let ry = ((oy as u64 * rot_h as u64) / out_h as u64).min(rot_h as u64 - 1) as u32;
        for ox in 0..out_w {
            let rx = ((ox as u64 * rot_w as u64) / out_w as u64).min(rot_w as u64 - 1) as u32;
            let (sx, sy) = unrotate(rx, ry, width, height, rot);
            // P010 stores 10-bit samples in the top 10 bits of each little-endian u16,
            // so the high byte (`[off + 1]`) directly gives the 8-bit sample value.
            let y_val = y_plane[sy as usize * y_stride + sx as usize * 2 + 1] as i32;
            let uv_off = (sy as usize / 2) * uv_stride + (sx as usize & !1) * 2;
            let u_val = uv_plane.get(uv_off + 1).copied().unwrap_or(128) as i32 - 128;
            let v_val = uv_plane.get(uv_off + 3).copied().unwrap_or(128) as i32 - 128;
            let (r, g, b) = yuv_to_rgb(y_val, u_val, v_val, true);
            let dst = ((oy as usize) * (out_w as usize) + (ox as usize)) * 3;
            rgb[dst] = r;
            rgb[dst + 1] = g;
            rgb[dst + 2] = b;
        }
    }

    encode_rgb_jpeg(&rgb, out_w, out_h)
}

#[inline]
fn unrotate(rx: u32, ry: u32, src_w: u32, src_h: u32, rot: u16) -> (u32, u32) {
    match rot {
        90 => (ry, src_h.saturating_sub(1).saturating_sub(rx)),
        180 => (src_w.saturating_sub(1).saturating_sub(rx), src_h.saturating_sub(1).saturating_sub(ry)),
        270 => (src_w.saturating_sub(1).saturating_sub(ry), rx),
        _ => (rx, ry),
    }
}

fn encode_rgb_jpeg(rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(16 * 1024);
    let mut enc = JpegEncoder::new_with_quality(&mut out, 80);
    enc.encode(rgb, width, height, ExtendedColorType::Rgb8).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Checks if a bitstream buffer starts with an Annex B NAL start code
/// (`00 00 01` or `00 00 00 01`). Always prefer [`valid_length_prefixed`] first
/// when `avcC`/`hvcC` declares length-prefixed framing, since a 256–511 byte
/// or 1-byte NAL length prefix starts with `00 00 01` / `00 00 00 01`.
pub fn is_annex_b(data: &[u8]) -> bool {
    data.starts_with(&[0, 0, 0, 1]) || data.starts_with(&[0, 0, 1])
}

/// Returns `true` if `packet` is a complete, structurally valid sequence of
/// `nal_len_size`-byte big-endian length-prefixed NAL units.
pub fn valid_length_prefixed(packet: &[u8], nal_len_size: usize) -> bool {
    if packet.is_empty() || !(1..=4).contains(&nal_len_size) {
        return false;
    }
    let mut off = 0usize;
    let mut nals = 0usize;
    while off < packet.len() {
        if off + nal_len_size > packet.len() {
            return false;
        }
        let mut len = 0usize;
        for &b in &packet[off..off + nal_len_size] {
            len = (len << 8) | b as usize;
        }
        off += nal_len_size;
        if len == 0 || off.checked_add(len).is_none_or(|end| end > packet.len()) {
            return false;
        }
        off += len;
        nals += 1;
    }
    nals > 0 && off == packet.len()
}

/// Returns the declared NAL length-prefix byte width (1..=4) from an `avcC` or
/// `hvcC` configuration record, or `None` if the extradata is Annex B or invalid.
#[cfg(any(target_os = "macos", test))]
pub fn nal_length_size_from_extradata(codec_is_hevc: bool, extra: &[u8]) -> Option<usize> {
    if extra.is_empty() || is_annex_b(extra) {
        return None;
    }
    if !codec_is_hevc {
        if extra.len() >= 7 && extra[0] == 1 {
            return Some(((extra[4] & 0x03) as usize) + 1);
        }
    } else if extra.len() >= 23 {
        return Some(((extra[21] & 0x03) as usize) + 1);
    }
    None
}

/// Converts an Annex B bitstream into 4-byte length-prefixed AVCC/HVCC NAL units.
#[cfg(any(target_os = "macos", test))]
pub fn annex_b_to_length_prefixed(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    let mut i = 0usize;
    let find_start = |buf: &[u8], mut p: usize| -> Option<(usize, usize)> {
        while p + 3 <= buf.len() {
            if buf[p] == 0 && buf[p + 1] == 0 {
                if buf[p + 2] == 1 {
                    return Some((p, 3));
                }
                if p + 4 <= buf.len() && buf[p + 2] == 0 && buf[p + 3] == 1 {
                    return Some((p, 4));
                }
            }
            p += 1;
        }
        None
    };
    while let Some((start, sc_len)) = find_start(data, i) {
        let nal_start = start + sc_len;
        let nal_end = find_start(data, nal_start).map_or(data.len(), |(p, _)| p);
        let nal = &data[nal_start..nal_end];
        if !nal.is_empty() && nal.len() <= u32::MAX as usize {
            out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            out.extend_from_slice(nal);
        }
        i = nal_end;
    }
    out
}

/// Extracts Annex B parameter sets (SPS/PPS/VPS) from an `avcC` or `hvcC`
/// configuration record (used by Windows Media Foundation).
/// Returns `(annex_b_params, nal_len_size, extradata_was_annex_b)`.
#[cfg(any(windows, test))]
pub fn extradata_to_annex_b(codec_is_hevc: bool, extra: &[u8]) -> (Vec<u8>, usize, bool) {
    if is_annex_b(extra) {
        return (extra.to_vec(), 4, true);
    }
    let mut out = Vec::new();
    if !codec_is_hevc {
        // ISO/IEC 14496-15 AVCDecoderConfigurationRecord
        if extra.len() < 7 || extra[0] != 1 {
            return (out, 4, false);
        }
        let nal_len_size = ((extra[4] & 0x03) as usize) + 1;
        let num_sps = (extra[5] & 0x1F) as usize;
        let mut off = 6usize;
        for _ in 0..num_sps {
            let Some(len_bytes) = extra.get(off..off + 2) else { break };
            let len = u16::from_be_bytes([len_bytes[0], len_bytes[1]]) as usize;
            off += 2;
            let Some(nal) = extra.get(off..off + len) else { break };
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(nal);
            off += len;
        }
        if let Some(&num_pps) = extra.get(off) {
            off += 1;
            for _ in 0..num_pps as usize {
                let Some(len_bytes) = extra.get(off..off + 2) else { break };
                let len = u16::from_be_bytes([len_bytes[0], len_bytes[1]]) as usize;
                off += 2;
                let Some(nal) = extra.get(off..off + len) else { break };
                out.extend_from_slice(&[0, 0, 0, 1]);
                out.extend_from_slice(nal);
                off += len;
            }
        }
        (out, nal_len_size, false)
    } else {
        // ISO/IEC 14496-15 HEVCDecoderConfigurationRecord
        if extra.len() < 23 {
            return (out, 4, false);
        }
        let nal_len_size = ((extra[21] & 0x03) as usize) + 1;
        let num_arrays = extra[22] as usize;
        let mut off = 23usize;
        for _ in 0..num_arrays {
            let Some(hdr) = extra.get(off..off + 3) else { break };
            let count = u16::from_be_bytes([hdr[1], hdr[2]]) as usize;
            off += 3;
            for _ in 0..count {
                let Some(lb) = extra.get(off..off + 2) else { break };
                let len = u16::from_be_bytes([lb[0], lb[1]]) as usize;
                off += 2;
                let Some(nal) = extra.get(off..off + len) else { break };
                out.extend_from_slice(&[0, 0, 0, 1]);
                out.extend_from_slice(nal);
                off += len;
            }
        }
        (out, nal_len_size, false)
    }
}

/// Converts length-prefixed AVCC/HVCC packet NAL units into Annex B format.
/// Only treats `packet` as already Annex B when `force_length_prefixed` is
/// false or the packet fails structural length-prefix validation.
#[cfg(any(windows, test))]
pub fn length_prefixed_to_annex_b(packet: &[u8], nal_len_size: usize, force_length_prefixed: bool) -> Vec<u8> {
    if !(1..=4).contains(&nal_len_size) {
        return packet.to_vec();
    }
    if (!force_length_prefixed || !valid_length_prefixed(packet, nal_len_size)) && is_annex_b(packet) {
        return packet.to_vec();
    }
    let mut out = Vec::with_capacity(packet.len() + 32);
    let mut off = 0usize;
    while off + nal_len_size <= packet.len() {
        let mut len = 0usize;
        for &b in &packet[off..off + nal_len_size] {
            len = (len << 8) | b as usize;
        }
        off += nal_len_size;
        let Some(nal) = packet.get(off..off + len) else { break };
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
        off += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_scales_rotates_and_encodes_valid_jpeg() {
        // 64x32 image: red on left half, blue on right half.
        let (w, h) = (64u32, 32u32);
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let off = ((y * w + x) * 4) as usize;
                if x < w / 2 {
                    bgra[off..off + 4].copy_from_slice(&[0, 0, 240, 255]); // red in BGRA
                } else {
                    bgra[off..off + 4].copy_from_slice(&[240, 0, 0, 255]); // blue in BGRA
                }
            }
        }
        let jpg = bgra_to_scaled_jpeg(&bgra, w, h, w as usize * 4, 0, 32).unwrap();
        let img = image::load_from_memory(&jpg).unwrap().to_rgb8();
        assert_eq!(img.dimensions(), (32, 16));
        assert!(img.get_pixel(4, 8)[0] > 180);
        assert!(img.get_pixel(28, 8)[2] > 180);

        // 90° clockwise rotation swaps dimensions (32x64 -> scaled width 16 => 16x32).
        let jpg90 = bgra_to_scaled_jpeg(&bgra, w, h, w as usize * 4, 90, 16).unwrap();
        let img90 = image::load_from_memory(&jpg90).unwrap().to_rgb8();
        assert_eq!(img90.dimensions(), (16, 32));
    }

    #[test]
    fn bottom_up_bgra_flips_vertically() {
        // Top half of buffer (y=0..8) is blue; bottom half (y=8..16) is red.
        // With bottom_up=true, top of output image must be red and bottom blue.
        let (w, h) = (16u32, 16u32);
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let off = ((y * w + x) * 4) as usize;
                if y < h / 2 {
                    bgra[off..off + 4].copy_from_slice(&[240, 0, 0, 255]); // blue
                } else {
                    bgra[off..off + 4].copy_from_slice(&[0, 0, 240, 255]); // red
                }
            }
        }
        let jpg = bgra_to_scaled_jpeg_oriented(&bgra, w, h, w as usize * 4, true, 0, 16).unwrap();
        let img = image::load_from_memory(&jpg).unwrap().to_rgb8();
        assert!(img.get_pixel(8, 2)[0] > 180, "top row should be red");
        assert!(img.get_pixel(8, 13)[2] > 180, "bottom row should be blue");
    }

    #[test]
    fn annex_b_round_trips_with_length_prefixed() {
        let annex = [0, 0, 0, 1, 0x65, 0xAA, 0xBB, 0, 0, 1, 0x06, 0xCC];
        let lp = annex_b_to_length_prefixed(&annex);
        assert_eq!(lp, [0, 0, 0, 3, 0x65, 0xAA, 0xBB, 0, 0, 0, 2, 0x06, 0xCC]);
        assert!(valid_length_prefixed(&lp, 4));
        let back = length_prefixed_to_annex_b(&lp, 4, true);
        assert_eq!(back, [0, 0, 0, 1, 0x65, 0xAA, 0xBB, 0, 0, 0, 1, 0x06, 0xCC]);
    }

    #[test]
    fn length_prefixed_300_byte_nal_is_not_misclassified_as_annex_b() {
        // 300 = 0x0000_012C -> starts with `00 00 01`, which collides with Annex B prefix.
        let mut pkt = Vec::with_capacity(4 + 300);
        pkt.extend_from_slice(&300u32.to_be_bytes());
        pkt.push(0x65);
        pkt.resize(4 + 300, 0xAB);
        assert!(is_annex_b(&pkt), "prefix intentionally collides with 3-byte start code");
        assert!(valid_length_prefixed(&pkt, 4));
        let annex = length_prefixed_to_annex_b(&pkt, 4, true);
        assert_eq!(&annex[..5], &[0, 0, 0, 1, 0x65]);
        assert_eq!(annex.len(), 4 + 300);
    }

    #[test]
    fn nv12_and_p010_scale_and_encode_valid_jpeg() {
        let (w, h) = (32u32, 16u32);
        let y = vec![200u8; (w * h) as usize];
        let uv = vec![128u8; (w * h / 2) as usize];
        let jpg = nv12_to_scaled_jpeg(&y, &uv, w, h, w as usize, w as usize, 0, 16).unwrap();
        let img = image::load_from_memory(&jpg).unwrap().to_rgb8();
        assert_eq!(img.dimensions(), (16, 8));
        let px = img.get_pixel(8, 4);
        assert!(px[0] > 180 && px[1] > 180 && px[2] > 180);

        // P010: high byte carries the 8-bit MSB of each 10-bit sample.
        let mut y10 = vec![0u8; (w * h * 2) as usize];
        for chunk in y10.as_chunks_mut::<2>().0 {
            chunk[1] = 200;
        }
        let mut uv10 = vec![0u8; (w * h) as usize];
        for chunk in uv10.as_chunks_mut::<2>().0 {
            chunk[1] = 128;
        }
        let jpg10 = p010_to_scaled_jpeg(&y10, &uv10, w, h, (w * 2) as usize, (w * 2) as usize, 0, 16).unwrap();
        let img10 = image::load_from_memory(&jpg10).unwrap().to_rgb8();
        assert_eq!(img10.dimensions(), (16, 8));
        let px10 = img10.get_pixel(8, 4);
        assert!(px10[0] > 180 && px10[1] > 180 && px10[2] > 180);
    }

    #[test]
    fn extradata_to_annex_b_extracts_avcc_sps_and_pps() {
        let avcc = [1, 0x64, 0, 0x1F, 0xFF, 0xE1, 0, 2, 0x67, 0x64, 1, 0, 2, 0x68, 0xEE];
        assert_eq!(nal_length_size_from_extradata(false, &avcc), Some(4));
        let (annex, nal_len, was_annex_b) = extradata_to_annex_b(false, &avcc);
        assert_eq!(nal_len, 4);
        assert!(!was_annex_b);
        assert_eq!(annex, [0, 0, 0, 1, 0x67, 0x64, 0, 0, 0, 1, 0x68, 0xEE]);
    }

    #[test]
    fn one_byte_nal_hvcc_extradata_rotations_and_truncated_buffers() {
        // 1-byte NAL starts with `00 00 00 01` (4-byte length prefix = 1), colliding with 4-byte Annex B start code.
        // Appending a second 2-byte NAL (`00 00 00 02 41 9A`) ensures real length-prefix conversion runs:
        // Annex B passthrough would leave `00 00 00 02` untouched instead of converting it to `00 00 00 01`.
        let two_nals = [0, 0, 0, 1, 0x09, 0, 0, 0, 2, 0x41, 0x9A];
        assert!(valid_length_prefixed(&two_nals, 4));
        assert_eq!(
            length_prefixed_to_annex_b(&two_nals, 4, true),
            [0, 0, 0, 1, 0x09, 0, 0, 0, 1, 0x41, 0x9A]
        );
        // When extradata itself is Annex B (`force_length_prefixed = false`), Annex B packets pass through untouched.
        assert_eq!(length_prefixed_to_annex_b(&two_nals, 4, false), two_nals);

        // Minimal hvcC record with 1 array containing 1 NAL (VPS).
        let mut hvcc = vec![0u8; 23];
        hvcc[0] = 1;
        hvcc[21] = 0xFF; // lengthSizeMinusOne = 3 -> 4 bytes
        hvcc[22] = 1; // numOfArrays = 1
        hvcc.extend_from_slice(&[0x20, 0, 1, 0, 2, 0x40, 0x01]);
        assert_eq!(nal_length_size_from_extradata(true, &hvcc), Some(4));
        let (annex, len_sz, was_ab) = extradata_to_annex_b(true, &hvcc);
        assert_eq!((len_sz, was_ab), (4, false));
        assert_eq!(annex, [0, 0, 0, 1, 0x40, 0x01]);

        // Asymmetric image (bright red in top-left quadrant, dark blue elsewhere) distinguishes
        // all four quarter-turn rotations (0°, 90°, 180°, 270°).
        let (w, h) = (32u32, 16u32);
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let off = ((y * w + x) * 4) as usize;
                if x < w / 2 && y < h / 2 {
                    bgra[off..off + 4].copy_from_slice(&[0, 0, 240, 255]); // red in BGRA
                } else {
                    bgra[off..off + 4].copy_from_slice(&[180, 10, 10, 255]); // blue in BGRA
                }
            }
        }
        let is_red = |img: &image::RgbImage, x: u32, y: u32| {
            let p = img.get_pixel(x, y);
            p[0] > 160 && p[2] < 80
        };
        let img0 = image::load_from_memory(&bgra_to_scaled_jpeg(&bgra, w, h, (w * 4) as usize, 0, 16).unwrap())
            .unwrap()
            .to_rgb8();
        assert_eq!(img0.dimensions(), (16, 8));
        assert!(is_red(&img0, 2, 2) && !is_red(&img0, 13, 5));

        let img90 = image::load_from_memory(&bgra_to_scaled_jpeg(&bgra, w, h, (w * 4) as usize, 90, 16).unwrap())
            .unwrap()
            .to_rgb8();
        assert_eq!(img90.dimensions(), (16, 32));
        // unrotate(90): sx = ry, sy = src_h - 1 - rx -> top-left source maps to bottom-left (rx near 15, ry near 0 -> top-right! wait: rx=13 => sy=2, ry=2 => sx=2).
        assert!(is_red(&img90, 13, 2) && !is_red(&img90, 2, 29));

        let img180 = image::load_from_memory(&bgra_to_scaled_jpeg(&bgra, w, h, (w * 4) as usize, 180, 16).unwrap())
            .unwrap()
            .to_rgb8();
        assert_eq!(img180.dimensions(), (16, 8));
        assert!(is_red(&img180, 13, 5) && !is_red(&img180, 2, 2));

        let img270 = image::load_from_memory(&bgra_to_scaled_jpeg(&bgra, w, h, (w * 4) as usize, 270, 16).unwrap())
            .unwrap()
            .to_rgb8();
        assert_eq!(img270.dimensions(), (16, 32));
        assert!(is_red(&img270, 2, 29) && !is_red(&img270, 13, 2));

        // Truncated buffers return Err cleanly without panicking.
        assert!(bgra_to_scaled_jpeg(&bgra[..16], w, h, (w * 4) as usize, 0, 16).is_err());
        assert!(nv12_to_scaled_jpeg(&[0u8; 4], &[0u8; 4], 32, 16, 32, 32, 0, 16).is_err());
        assert!(p010_to_scaled_jpeg(&[0u8; 4], &[0u8; 4], 32, 16, 64, 64, 0, 16).is_err());
    }
}
