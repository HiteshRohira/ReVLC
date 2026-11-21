//! Linux native preview hook.
//!
//! Raw VA-API slice-header submission requires full H.264/HEVC/AV1 bitstream
//! parsers; Linux instead keeps the distribution's `ffmpeg` package dependency
//! (zero bundled binary size) and delegates directly to `keyframes.rs` (which
//! probes NVDEC and VA-API render nodes automatically) with libVLC fallback.

use crate::container::VideoTrackIndex;

pub struct NativeDecoder;

impl NativeDecoder {
    pub fn open(_track: &VideoTrackIndex) -> Result<Self, String> {
        Err("Linux delegates keyframe decoding to VA-API/NVDEC ffmpeg fallback".into())
    }

    pub fn decode_keyframe(&mut self, _packet: &[u8], _target_width: u32) -> Result<Vec<u8>, String> {
        Err("Linux delegates keyframe decoding to fallback".into())
    }
}
