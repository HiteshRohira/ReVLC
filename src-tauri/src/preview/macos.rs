//! macOS warm-session hardware keyframe decoder via VideoToolbox.
//!
//! Creates a single `VTDecompressionSessionRef` per active `(file, track)`
//! and reuses it across all keyframe decodes for H.264, HEVC, VP9, and AV1.
//! Decoded frames are converted in hardware to 32-bit BGRA, downscaled in
//! memory, and JPEG-encoded without touching the filesystem or spawning FFmpeg.

use std::ffi::{c_char, c_void};
use std::ptr::{null, null_mut};
use std::sync::Mutex;

use crate::container::{VideoCodec, VideoTrackIndex};

use super::scale;

type OSStatus = i32;
type CFAllocatorRef = *const c_void;
type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFDataRef = *const c_void;
type CFNumberRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CMFormatDescriptionRef = *const c_void;
type CMBlockBufferRef = *mut c_void;
type CMSampleBufferRef = *mut c_void;
type CVPixelBufferRef = *mut c_void;
type CVImageBufferRef = *mut c_void;
type VTDecompressionSessionRef = *mut c_void;
type CMVideoCodecType = u32;

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const K_CF_NUMBER_SINT32_TYPE: isize = 3;
const K_CV_PIXEL_FORMAT_32BGRA: i32 = 0x4247_5241; // 'BGRA'
const K_CV_PIXEL_BUFFER_LOCK_READ_ONLY: u64 = 1;

const CODEC_H264: CMVideoCodecType = u32::from_be_bytes(*b"avc1");
const CODEC_HEVC: CMVideoCodecType = u32::from_be_bytes(*b"hvc1");
const CODEC_VP9: CMVideoCodecType = u32::from_be_bytes(*b"vp09");
const CODEC_AV1: CMVideoCodecType = u32::from_be_bytes(*b"av01");

#[repr(C)]
#[derive(Clone, Copy)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CMSampleTimingInfo {
    duration: CMTime,
    presentation_time_stamp: CMTime,
    decode_time_stamp: CMTime,
}

type VTDecompressionOutputCallback = unsafe extern "C" fn(
    decompression_output_ref_con: *mut c_void,
    source_frame_ref_con: *mut c_void,
    status: OSStatus,
    info_flags: u32,
    image_buffer: CVImageBufferRef,
    presentation_time_stamp: CMTime,
    presentation_duration: CMTime,
);

#[repr(C)]
struct VTDecompressionOutputCallbackRecord {
    callback: VTDecompressionOutputCallback,
    ref_con: *mut c_void,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    fn CFRelease(cf: CFTypeRef);
    fn CFStringCreateWithBytes(
        alloc: CFAllocatorRef,
        bytes: *const u8,
        num_bytes: isize,
        encoding: u32,
        is_external: u8,
    ) -> CFStringRef;
    fn CFDataCreate(alloc: CFAllocatorRef, bytes: *const u8, length: isize) -> CFDataRef;
    fn CFNumberCreate(alloc: CFAllocatorRef, the_type: isize, value_ptr: *const c_void) -> CFNumberRef;
    fn CFDictionaryCreate(
        allocator: CFAllocatorRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        num_values: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFDictionaryRef;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: CFStringRef;
    fn CVPixelBufferLockBaseAddress(pixel_buffer: CVPixelBufferRef, lock_flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pixel_buffer: CVPixelBufferRef, unlock_flags: u64) -> i32;
    fn CVPixelBufferGetWidth(pixel_buffer: CVPixelBufferRef) -> usize;
    fn CVPixelBufferGetHeight(pixel_buffer: CVPixelBufferRef) -> usize;
    fn CVPixelBufferGetBytesPerRow(pixel_buffer: CVPixelBufferRef) -> usize;
    fn CVPixelBufferGetBaseAddress(pixel_buffer: CVPixelBufferRef) -> *const c_void;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    static kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms: CFStringRef;
    fn CMVideoFormatDescriptionCreate(
        allocator: CFAllocatorRef,
        codec_type: CMVideoCodecType,
        width: i32,
        height: i32,
        extensions: CFDictionaryRef,
        format_description_out: *mut CMFormatDescriptionRef,
    ) -> OSStatus;
    fn CMBlockBufferCreateWithMemoryBlock(
        structure_allocator: CFAllocatorRef,
        memory_block: *mut c_void,
        block_length: usize,
        block_allocator: CFAllocatorRef,
        custom_block_source: *const c_void,
        offset_to_data: usize,
        data_length: usize,
        flags: u32,
        block_buffer_out: *mut CMBlockBufferRef,
    ) -> OSStatus;
    fn CMBlockBufferReplaceDataBytes(
        source_bytes: *const c_void,
        destination_buffer: CMBlockBufferRef,
        offset_into_destination: usize,
        data_length: usize,
    ) -> OSStatus;
    fn CMSampleBufferCreateReady(
        allocator: CFAllocatorRef,
        data_buffer: CMBlockBufferRef,
        format_description: CMFormatDescriptionRef,
        num_samples: isize,
        num_sample_timing_entries: isize,
        sample_timing_array: *const CMSampleTimingInfo,
        num_sample_size_entries: isize,
        sample_size_array: *const usize,
        sample_buffer_out: *mut CMSampleBufferRef,
    ) -> OSStatus;
}

#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    fn VTDecompressionSessionCreate(
        allocator: CFAllocatorRef,
        video_format_description: CMFormatDescriptionRef,
        video_decoder_specification: CFDictionaryRef,
        destination_image_buffer_attributes: CFDictionaryRef,
        output_callback: *const VTDecompressionOutputCallbackRecord,
        decompression_session_out: *mut VTDecompressionSessionRef,
    ) -> OSStatus;
    fn VTDecompressionSessionDecodeFrame(
        session: VTDecompressionSessionRef,
        sample_buffer: CMSampleBufferRef,
        decode_flags: u32,
        source_frame_ref_con: *mut c_void,
        info_flags_out: *mut u32,
    ) -> OSStatus;
    fn VTDecompressionSessionWaitForAsynchronousFrames(session: VTDecompressionSessionRef) -> OSStatus;
    fn VTDecompressionSessionInvalidate(session: VTDecompressionSessionRef);
}

/// Registers macOS supplemental decoders (VP9 on macOS 11+, AV1 on macOS 13+)
/// dynamically so older SDK/OS builds never fail to link.
fn register_supplemental_decoder(codec: CMVideoCodecType) {
    type RegisterFn = unsafe extern "C" fn(CMVideoCodecType);
    // SAFETY: dlsym on RTLD_DEFAULT for Apple's VideoToolbox symbol.
    unsafe {
        let sym = libc::dlsym(libc::RTLD_DEFAULT, c"VTRegisterSupplementalVideoDecoderIfAvailable".as_ptr() as *const c_char);
        if !sym.is_null() {
            let func: RegisterFn = std::mem::transmute(sym);
            func(codec);
        }
    }
}

/// RAII wrapper around a retained CoreFoundation object.
struct CfGuard(CFTypeRef);

impl CfGuard {
    fn new(ptr: CFTypeRef) -> Result<Self, String> {
        if ptr.is_null() { Err("CoreFoundation allocation failed".into()) } else { Ok(Self(ptr)) }
    }
}

impl Drop for CfGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` is an owned CF object created by a Create rule function.
            unsafe { CFRelease(self.0) };
        }
    }
}

struct DecodeTarget {
    target_width: u32,
    rotation: u16,
    result: Mutex<Option<Result<Vec<u8>, String>>>,
}

unsafe extern "C" fn on_decoded_frame(
    _decompression_output_ref_con: *mut c_void,
    source_frame_ref_con: *mut c_void,
    status: OSStatus,
    _info_flags: u32,
    image_buffer: CVImageBufferRef,
    _pts: CMTime,
    _duration: CMTime,
) {
    if source_frame_ref_con.is_null() {
        return;
    }
    // SAFETY: `source_frame_ref_con` points to the `DecodeTarget` kept alive on
    // the caller's thread until `VTDecompressionSessionWaitForAsynchronousFrames` returns.
    let target = unsafe { &*(source_frame_ref_con as *const DecodeTarget) };
    let outcome = if status != 0 {
        Err(format!("VideoToolbox decode error ({status})"))
    } else if image_buffer.is_null() {
        Err("VideoToolbox returned null pixel buffer".into())
    } else {
        unsafe {
            if CVPixelBufferLockBaseAddress(image_buffer, K_CV_PIXEL_BUFFER_LOCK_READ_ONLY) != 0 {
                Err("CVPixelBufferLockBaseAddress failed".into())
            } else {
                let w = CVPixelBufferGetWidth(image_buffer) as u32;
                let h = CVPixelBufferGetHeight(image_buffer) as u32;
                let stride = CVPixelBufferGetBytesPerRow(image_buffer);
                let base = CVPixelBufferGetBaseAddress(image_buffer) as *const u8;
                let res = if base.is_null() || w == 0 || h == 0 || stride < (w as usize) * 4 {
                    Err("invalid VideoToolbox pixel buffer dimensions".into())
                } else {
                    let bytes = std::slice::from_raw_parts(base, stride * (h as usize));
                    scale::bgra_to_scaled_jpeg(bytes, w, h, stride, target.rotation, target.target_width)
                };
                CVPixelBufferUnlockBaseAddress(image_buffer, K_CV_PIXEL_BUFFER_LOCK_READ_ONLY);
                res
            }
        }
    };
    *target.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
}

/// Warm VideoToolbox decompression session bound to one video track.
pub struct NativeDecoder {
    session: VTDecompressionSessionRef,
    format_desc: CMFormatDescriptionRef,
    codec: VideoCodec,
    nal_len_size: Option<usize>,
    rotation: u16,
    frame_counter: i64,
}

// SAFETY: A single preview worker thread owns and drives each `NativeDecoder`
// instance sequentially; VideoToolbox sessions are thread-safe to move across threads.
unsafe impl Send for NativeDecoder {}

impl NativeDecoder {
    pub fn open(track: &VideoTrackIndex) -> Result<Self, String> {
        if track.width == 0 || track.height == 0 {
            return Err("unknown video dimensions".into());
        }
        let (codec_type, atom_name): (CMVideoCodecType, &[u8]) = match track.codec {
            VideoCodec::H264 => (CODEC_H264, b"avcC"),
            VideoCodec::Hevc => (CODEC_HEVC, b"hvcC"),
            VideoCodec::Vp9 => {
                register_supplemental_decoder(CODEC_VP9);
                (CODEC_VP9, b"vpcC")
            }
            VideoCodec::Av1 => {
                register_supplemental_decoder(CODEC_AV1);
                (CODEC_AV1, b"av1C")
            }
            VideoCodec::Other(ref name) => return Err(format!("codec {name} not supported by VideoToolbox previewer")),
        };

        // Build CMFormatDescription extensions dictionary with the codec atom.
        let format_desc = unsafe {
            let extensions = if !track.extradata.is_empty() {
                let atom_key = CfGuard::new(CFStringCreateWithBytes(
                    null(),
                    atom_name.as_ptr(),
                    atom_name.len() as isize,
                    K_CF_STRING_ENCODING_UTF8,
                    0,
                ))?;
                let atom_data = CfGuard::new(CFDataCreate(
                    null(),
                    track.extradata.as_ptr(),
                    track.extradata.len() as isize,
                ))?;
                let atoms_dict = CfGuard::new(CFDictionaryCreate(
                    null(),
                    &atom_key.0,
                    &atom_data.0,
                    1,
                    &kCFTypeDictionaryKeyCallBacks,
                    &kCFTypeDictionaryValueCallBacks,
                ))?;
                let ext_key = kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms;
                Some(CfGuard::new(CFDictionaryCreate(
                    null(),
                    &ext_key,
                    &atoms_dict.0,
                    1,
                    &kCFTypeDictionaryKeyCallBacks,
                    &kCFTypeDictionaryValueCallBacks,
                ))?)
            } else if matches!(track.codec, VideoCodec::Vp9 | VideoCodec::Av1) {
                None
            } else {
                return Err("missing avcC/hvcC extradata for VideoToolbox".into());
            };

            let mut desc: CMFormatDescriptionRef = null();
            let status = CMVideoFormatDescriptionCreate(
                null(),
                codec_type,
                track.width as i32,
                track.height as i32,
                extensions.as_ref().map_or(null(), |e| e.0),
                &mut desc,
            );
            if status != 0 || desc.is_null() {
                return Err(format!("CMVideoFormatDescriptionCreate failed ({status})"));
            }
            desc
        };

        let session = unsafe {
            let pix_fmt = K_CV_PIXEL_FORMAT_32BGRA;
            let pix_num = match CfGuard::new(CFNumberCreate(null(), K_CF_NUMBER_SINT32_TYPE, &pix_fmt as *const i32 as *const c_void)) {
                Ok(n) => n,
                Err(e) => {
                    CFRelease(format_desc);
                    return Err(e);
                }
            };
            let dest_key = kCVPixelBufferPixelFormatTypeKey;
            let dest_attrs = match CfGuard::new(CFDictionaryCreate(
                null(),
                &dest_key,
                &pix_num.0,
                1,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            )) {
                Ok(d) => d,
                Err(e) => {
                    CFRelease(format_desc);
                    return Err(e);
                }
            };
            let cb = VTDecompressionOutputCallbackRecord {
                callback: on_decoded_frame,
                ref_con: null_mut(),
            };
            let mut session: VTDecompressionSessionRef = null_mut();
            let status = VTDecompressionSessionCreate(null(), format_desc, null(), dest_attrs.0, &cb, &mut session);
            if status != 0 || session.is_null() {
                CFRelease(format_desc);
                return Err(format!("VTDecompressionSessionCreate failed ({status})"));
            }
            session
        };

        let nal_len_size = match track.codec {
            VideoCodec::H264 => scale::nal_length_size_from_extradata(false, &track.extradata),
            VideoCodec::Hevc => scale::nal_length_size_from_extradata(true, &track.extradata),
            _ => None,
        };

        Ok(Self {
            session,
            format_desc,
            codec: track.codec.clone(),
            nal_len_size,
            rotation: track.rotation,
            frame_counter: 0,
        })
    }

    pub fn decode_keyframe(&mut self, packet: &[u8], target_width: u32) -> Result<Vec<u8>, String> {
        if packet.is_empty() {
            return Err("empty keyframe packet".into());
        }
        // Standard MP4 and MKV H.264/HEVC tracks are already length-prefixed;
        // only convert Annex B when length-prefix framing is absent or invalid,
        // so a 256–511 byte NAL (`00 00 01 XX`) is never misclassified as Annex B.
        let owned_converted;
        let bytes: &[u8] = if matches!(self.codec, VideoCodec::H264 | VideoCodec::Hevc)
            && self.nal_len_size.is_none_or(|n| !scale::valid_length_prefixed(packet, n))
            && scale::is_annex_b(packet)
        {
            owned_converted = scale::annex_b_to_length_prefixed(packet);
            &owned_converted
        } else {
            packet
        };
        if bytes.is_empty() {
            return Err("empty converted keyframe packet".into());
        }

        self.frame_counter += 1;
        let timing = CMSampleTimingInfo {
            duration: CMTime { value: 1, timescale: 30, flags: 1, epoch: 0 },
            presentation_time_stamp: CMTime { value: self.frame_counter, timescale: 30, flags: 1, epoch: 0 },
            decode_time_stamp: CMTime { value: self.frame_counter, timescale: 30, flags: 1, epoch: 0 },
        };
        let sample_size = bytes.len();

        let target = DecodeTarget {
            target_width,
            rotation: self.rotation,
            result: Mutex::new(None),
        };

        unsafe {
            // Allocate owned CMBlockBuffer memory so VideoToolbox never borrows
            // our Rust slice past the decode call even on error paths.
            let mut block_buf: CMBlockBufferRef = null_mut();
            let st = CMBlockBufferCreateWithMemoryBlock(
                null(),
                null_mut(),
                sample_size,
                null(),
                null(),
                0,
                sample_size,
                0,
                &mut block_buf,
            );
            if st != 0 || block_buf.is_null() {
                return Err(format!("CMBlockBufferCreate failed ({st})"));
            }
            let _bb_guard = CfGuard(block_buf as CFTypeRef);
            let st = CMBlockBufferReplaceDataBytes(bytes.as_ptr() as *const c_void, block_buf, 0, sample_size);
            if st != 0 {
                return Err(format!("CMBlockBufferReplaceDataBytes failed ({st})"));
            }

            let mut sample_buf: CMSampleBufferRef = null_mut();
            let st = CMSampleBufferCreateReady(
                null(),
                block_buf,
                self.format_desc,
                1,
                1,
                &timing,
                1,
                &sample_size,
                &mut sample_buf,
            );
            if st != 0 || sample_buf.is_null() {
                return Err(format!("CMSampleBufferCreateReady failed ({st})"));
            }
            let _sb_guard = CfGuard(sample_buf as CFTypeRef);

            let mut info_flags: u32 = 0;
            let st = VTDecompressionSessionDecodeFrame(
                self.session,
                sample_buf,
                0,
                (&target as *const DecodeTarget) as *mut c_void,
                &mut info_flags,
            );
            // Always wait before dropping `target` so no callback can ever
            // dereference `source_frame_ref_con` after this function returns.
            let _ = VTDecompressionSessionWaitForAsynchronousFrames(self.session);
            if st != 0 {
                return Err(format!("VTDecompressionSessionDecodeFrame failed ({st})"));
            }
        }

        let outcome = target.result.into_inner().unwrap_or_else(|e| e.into_inner());
        outcome.unwrap_or_else(|| Err("VideoToolbox produced no output frame".into()))
    }
}

impl Drop for NativeDecoder {
    fn drop(&mut self) {
        // SAFETY: `self.session` and `self.format_desc` are valid owned handles.
        unsafe {
            let _ = VTDecompressionSessionWaitForAsynchronousFrames(self.session);
            VTDecompressionSessionInvalidate(self.session);
            CFRelease(self.session as CFTypeRef);
            CFRelease(self.format_desc as CFTypeRef);
        }
    }
}
