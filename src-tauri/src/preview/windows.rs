//! Windows warm-session keyframe decoder via Media Foundation + D3D11VA.
//!
//! Enumerates synchronous `IMFTransform` video decoders (H.264, HEVC, VP9, AV1),
//! attaches an `IMFDXGIDeviceManager` backed by a D3D11 device when supported
//! (Microsoft's built-in synchronous MFTs use D3D11VA hardware acceleration when
//! given a device manager, and fall back transparently to software decode when
//! D3D11 is unavailable), and reuses one warm session across keyframes.
//!
//! # Threading invariant
//! `NativeDecoder` is intentionally `!Send`: it initializes COM (`CoInitializeEx`)
//! on the calling preview worker thread and must be opened, driven, and dropped
//! on that same thread.

use std::mem::ManuallyDrop;

use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
    ID3D11Device, ID3D11Multithread,
};
use windows::Win32::Graphics::Dxgi::IDXGIAdapter;
use windows::Win32::Media::MediaFoundation::{
    IMF2DBuffer2, IMFActivate, IMFDXGIDeviceManager, IMFMediaBuffer, IMFMediaType, IMFSample, IMFShutdown,
    IMFTransform, MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE, MF_MT_DEFAULT_STRIDE,
    MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_VERSION, MF2DBuffer_LockFlags_Read,
    MFCreateDXGIDeviceManager, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video,
    MFShutdown, MFStartup, MFT_CATEGORY_VIDEO_DECODER, MFT_ENUM_FLAG_LOCALMFT, MFT_ENUM_FLAG_SORTANDFILTER,
    MFT_ENUM_FLAG_SYNCMFT, MFT_MESSAGE_COMMAND_DRAIN, MFT_MESSAGE_COMMAND_FLUSH,
    MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_MESSAGE_SET_D3D_MANAGER,
    MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO, MFTEnumEx,
    MFVideoFormat_AV1, MFVideoFormat_H264, MFVideoFormat_HEVC, MFVideoFormat_NV12, MFVideoFormat_P010,
    MFVideoFormat_RGB32, MFVideoFormat_VP90, MFSTARTUP_NOSOCKET,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize};
use windows::core::{GUID, Interface};

use crate::container::{VideoCodec, VideoTrackIndex};

use super::scale;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputFormat {
    Nv12,
    P010,
    Rgb32,
}

/// RAII guard balancing `CoInitializeEx` and `MFStartup`.
/// Stored as the last field of `NativeDecoder` so all COM/MF interfaces drop
/// before `MFShutdown` and `CoUninitialize` run.
struct MfGuard {
    com_inited: bool,
    mf_started: bool,
}

impl MfGuard {
    fn start() -> Result<Self, String> {
        // SAFETY: Balanced with `CoUninitialize` in `Drop` on the same thread.
        let com_inited = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        // SAFETY: Balanced with `MFShutdown` in `Drop`.
        if let Err(e) = unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) } {
            if com_inited {
                // SAFETY: `CoInitializeEx` succeeded above on this thread.
                unsafe { CoUninitialize() };
            }
            return Err(format!("MFStartup failed: {e}"));
        }
        Ok(Self { com_inited, mf_started: true })
    }
}

impl Drop for MfGuard {
    fn drop(&mut self) {
        if self.mf_started {
            // SAFETY: Matches `MFStartup` in `MfGuard::start`.
            let _ = unsafe { MFShutdown() };
        }
        if self.com_inited {
            // SAFETY: Matches `CoInitializeEx` on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

struct Lock2DGuard<'a>(&'a IMF2DBuffer2);
impl Drop for Lock2DGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: Balances a successful `Lock2DSize` call.
        let _ = unsafe { self.0.Unlock2D() };
    }
}

struct Lock1DGuard<'a>(&'a IMFMediaBuffer);
impl Drop for Lock1DGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: Balances a successful `IMFMediaBuffer::Lock` call.
        let _ = unsafe { self.0.Unlock() };
    }
}

/// Warm Media Foundation / D3D11VA keyframe decoder session for one video track.
pub struct NativeDecoder {
    transform: IMFTransform,
    _d3d_manager: Option<IMFDXGIDeviceManager>,
    _d3d_device: Option<ID3D11Device>,
    codec: VideoCodec,
    width: u32,
    height: u32,
    /// The container's picture size. Media Foundation's frame size can be
    /// padded to whole macroblocks (54 rows become 64), so frames are cropped
    /// to this before scaling.
    visible_width: u32,
    visible_height: u32,
    stride: i32,
    rotation: u16,
    output_format: OutputFormat,
    annex_b_params: Vec<u8>,
    nal_len_size: usize,
    extradata_is_annex_b: bool,
    sample_idx: i64,
    // Must be dropped LAST, after all COM objects above are released.
    _mf_guard: MfGuard,
}

impl NativeDecoder {
    pub fn open(track: &VideoTrackIndex) -> Result<Self, String> {
        if track.width == 0 || track.height == 0 {
            return Err("unknown video dimensions".into());
        }
        let subtype: GUID = match track.codec {
            VideoCodec::H264 => MFVideoFormat_H264,
            VideoCodec::Hevc => MFVideoFormat_HEVC,
            VideoCodec::Vp9 => MFVideoFormat_VP90,
            VideoCodec::Av1 => MFVideoFormat_AV1,
            VideoCodec::Other(ref name) => return Err(format!("codec {name} not supported by Media Foundation previewer")),
        };

        let (annex_b_params, nal_len_size, extradata_is_annex_b) = match track.codec {
            VideoCodec::H264 => scale::extradata_to_annex_b(false, &track.extradata),
            VideoCodec::Hevc => scale::extradata_to_annex_b(true, &track.extradata),
            _ => (Vec::new(), 4, false),
        };

        let mf_guard = MfGuard::start()?;

        // Enumerate synchronous MFTs (Microsoft's built-in decoders still use
        // D3D11VA GPU decoding when given an IMFDXGIDeviceManager) so we never
        // pick an async-only MFT that fails with MF_E_TRANSFORM_ASYNC_LOCKED.
        let input_info = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: subtype,
        };
        let flags = MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER;
        let mut activates_ptr: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        // SAFETY: Output pointers are valid stack locals; returned array is freed below.
        unsafe {
            MFTEnumEx(MFT_CATEGORY_VIDEO_DECODER, flags, Some(&input_info), None, &mut activates_ptr, &mut count)
        }
        .map_err(|e| format!("MFTEnumEx failed: {e}"))?;

        let mut candidates: Vec<IMFActivate> = Vec::new();
        if !activates_ptr.is_null() {
            for i in 0..count as usize {
                // SAFETY: `MFTEnumEx` populated `count` owned `Option<IMFActivate>` slots.
                if let Some(act) = unsafe { std::ptr::read(activates_ptr.add(i)) } {
                    candidates.push(act);
                }
            }
            // SAFETY: Array was allocated by `MFTEnumEx` via CoTaskMemAlloc.
            unsafe { CoTaskMemFree(Some(activates_ptr as *const _)) };
        }
        if candidates.is_empty() {
            return Err("no synchronous Media Foundation decoder MFT found for codec".into());
        }

        let mut last_err = String::from("failed to initialize any Media Foundation decoder candidate");
        for act in candidates {
            // Try hardware-accelerated D3D11VA first, then software on the same candidate.
            for try_d3d in [true, false] {
                let transform: IMFTransform = match unsafe { act.ActivateObject::<IMFTransform>() } {
                    Ok(mft) => mft,
                    Err(e) => {
                        last_err = format!("ActivateObject failed: {e}");
                        break;
                    }
                };
                let (d3d_device, d3d_manager) = if try_d3d {
                    create_d3d11_manager(&transform)
                } else {
                    (None, None)
                };
                if try_d3d && d3d_manager.is_none() {
                    // D3D11 wasn't accepted; configure this same instance in software mode below.
                    if let Ok(out_cfg) = negotiate_types(&transform, subtype, track.width, track.height) {
                        return Ok(Self {
                            transform,
                            _d3d_manager: None,
                            _d3d_device: None,
                            codec: track.codec.clone(),
                            width: out_cfg.width,
                            height: out_cfg.height,
                            visible_width: track.width,
                            visible_height: track.height,
                            stride: out_cfg.stride,
                            rotation: track.rotation,
                            output_format: out_cfg.format,
                            annex_b_params,
                            nal_len_size,
                            extradata_is_annex_b,
                            sample_idx: 0,
                            _mf_guard: mf_guard,
                        });
                    }
                    shutdown_mft(&transform);
                    break;
                }
                match negotiate_types(&transform, subtype, track.width, track.height) {
                    Ok(out_cfg) => {
                        return Ok(Self {
                            transform,
                            _d3d_manager: d3d_manager,
                            _d3d_device: d3d_device,
                            codec: track.codec.clone(),
                            width: out_cfg.width,
                            height: out_cfg.height,
                            visible_width: track.width,
                            visible_height: track.height,
                            stride: out_cfg.stride,
                            rotation: track.rotation,
                            output_format: out_cfg.format,
                            annex_b_params,
                            nal_len_size,
                            extradata_is_annex_b,
                            sample_idx: 0,
                            _mf_guard: mf_guard,
                        });
                    }
                    Err(e) => {
                        last_err = e;
                        shutdown_mft(&transform);
                    }
                }
            }
        }

        Err(last_err)
    }

    pub fn decode_keyframe(&mut self, packet: &[u8], target_width: u32) -> Result<Vec<u8>, String> {
        if packet.is_empty() {
            return Err("empty keyframe packet".into());
        }
        let input_bytes: Vec<u8> = match self.codec {
            VideoCodec::H264 | VideoCodec::Hevc => {
                let mut v = Vec::with_capacity(self.annex_b_params.len() + packet.len() + 32);
                v.extend_from_slice(&self.annex_b_params);
                v.extend_from_slice(&scale::length_prefixed_to_annex_b(
                    packet,
                    self.nal_len_size,
                    !self.extradata_is_annex_b,
                ));
                v
            }
            _ => packet.to_vec(),
        };
        let input_len = u32::try_from(input_bytes.len()).map_err(|_| "packet too large")?;

        // SAFETY: Valid COM MFT calls on the thread that initialized MF.
        unsafe {
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
            let buf = MFCreateMemoryBuffer(input_len).map_err(|e| e.to_string())?;
            let mut dst_ptr: *mut u8 = std::ptr::null_mut();
            buf.Lock(&mut dst_ptr, None, None).map_err(|e| e.to_string())?;
            {
                let _lock = Lock1DGuard(&buf);
                if dst_ptr.is_null() {
                    return Err("MFCreateMemoryBuffer returned null pointer".into());
                }
                std::ptr::copy_nonoverlapping(input_bytes.as_ptr(), dst_ptr, input_bytes.len());
            }
            buf.SetCurrentLength(input_len).map_err(|e| e.to_string())?;

            let sample: IMFSample = MFCreateSample().map_err(|e| e.to_string())?;
            sample.AddBuffer(&buf).map_err(|e| e.to_string())?;
            self.sample_idx += 1;
            let _ = sample.SetSampleTime(self.sample_idx * 333_333);
            let _ = sample.SetSampleDuration(333_333);

            self.transform.ProcessInput(0, &sample, 0).map_err(|e| format!("ProcessInput failed: {e}"))?;
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0);

            for _ in 0..8 {
                let info = self.transform.GetOutputStreamInfo(0).map_err(|e| e.to_string())?;
                let provides = (info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32) != 0;
                let out_sample = if provides {
                    None
                } else {
                    let s = MFCreateSample().map_err(|e| e.to_string())?;
                    let alloc_bytes = info.cbSize.max(self.width.saturating_mul(self.height).saturating_mul(4));
                    let b = MFCreateMemoryBuffer(alloc_bytes).map_err(|e| e.to_string())?;
                    s.AddBuffer(&b).map_err(|e| e.to_string())?;
                    Some(s)
                };
                let mut out_buf = [MFT_OUTPUT_DATA_BUFFER {
                    dwStreamID: 0,
                    pSample: ManuallyDrop::new(out_sample),
                    dwStatus: 0,
                    pEvents: ManuallyDrop::new(None),
                }];
                let mut status = 0u32;
                let res = self.transform.ProcessOutput(0, &mut out_buf, &mut status);
                // Take ownership back from ManuallyDrop so COM refs never leak.
                let produced = ManuallyDrop::take(&mut out_buf[0].pSample);
                let _events = ManuallyDrop::take(&mut out_buf[0].pEvents);

                match res {
                    Ok(()) => {
                        let Some(out_s) = produced else { continue };
                        let media_buf = out_s.ConvertToContiguousBuffer().map_err(|e| e.to_string())?;
                        return self.encode_media_buffer(&media_buf, target_width);
                    }
                    Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                        let cfg = configure_output_type(&self.transform, self.width, self.height)?;
                        self.output_format = cfg.format;
                        self.width = cfg.width;
                        self.height = cfg.height;
                        self.stride = cfg.stride;
                    }
                    Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
                    Err(e) => return Err(format!("ProcessOutput failed: {e}")),
                }
            }
            Err("Media Foundation produced no output frame".into())
        }
    }

    fn encode_media_buffer(&self, media_buf: &IMFMediaBuffer, target_width: u32) -> Result<Vec<u8>, String> {
        let w = self.width;
        let h = self.height;

        // Prefer IMF2DBuffer2::Lock2DSize so we get exact buffer start, byte length,
        // scanline0 pointer, and signed pitch without guessing memory bounds.
        if let Ok(buf2d) = media_buf.cast::<IMF2DBuffer2>() {
            let mut scanline0: *mut u8 = std::ptr::null_mut();
            let mut pitch: i32 = 0;
            let mut buf_start: *mut u8 = std::ptr::null_mut();
            let mut buf_len: u32 = 0;
            // SAFETY: Output pointers are valid locals; guard unlocks on all exit paths.
            if unsafe {
                buf2d.Lock2DSize(
                    MF2DBuffer_LockFlags_Read,
                    &mut scanline0,
                    &mut pitch,
                    &mut buf_start,
                    &mut buf_len,
                )
            }
            .is_ok()
            {
                let _guard = Lock2DGuard(&buf2d);
                if !buf_start.is_null() && buf_len > 0 && pitch != 0 {
                    // SAFETY: `Lock2DSize` guarantees `buf_start..buf_start + buf_len` is valid for reading.
                    let full = unsafe { std::slice::from_raw_parts(buf_start, buf_len as usize) };
                    return self.encode_bounded_slice(full, scanline0 as usize, buf_start as usize, pitch, w, h, target_width);
                }
            }
        }

        let mut ptr: *mut u8 = std::ptr::null_mut();
        let mut cur_len = 0u32;
        // SAFETY: Valid COM buffer lock; RAII guard unlocks on all exit paths.
        unsafe { media_buf.Lock(&mut ptr, None, Some(&mut cur_len)) }.map_err(|e| e.to_string())?;
        let _guard = Lock1DGuard(media_buf);
        if ptr.is_null() || cur_len == 0 {
            return Err("empty Media Foundation output buffer".into());
        }
        // SAFETY: `IMFMediaBuffer::Lock` guarantees `cur_len` readable bytes at `ptr`.
        let slice = unsafe { std::slice::from_raw_parts(ptr, cur_len as usize) };
        let pitch = if self.stride != 0 {
            self.stride
        } else {
            match self.output_format {
                OutputFormat::Nv12 => w as i32,
                OutputFormat::P010 => (w as i32).saturating_mul(2),
                OutputFormat::Rgb32 => (w as i32).saturating_mul(4),
            }
        };
        let scanline0 = if pitch < 0 {
            let abs_stride = pitch.unsigned_abs() as usize;
            let last_row = abs_stride.checked_mul((h as usize).saturating_sub(1)).ok_or("stride overflow")?;
            (ptr as usize).checked_add(last_row).ok_or("pointer overflow")?
        } else {
            ptr as usize
        };
        self.encode_bounded_slice(slice, scanline0, ptr as usize, pitch, w, h, target_width)
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_bounded_slice(
        &self,
        full: &[u8],
        scanline0: usize,
        buf_start: usize,
        pitch: i32,
        w: u32,
        h: u32,
        target_width: u32,
    ) -> Result<Vec<u8>, String> {
        let stride = pitch.unsigned_abs() as usize;
        let bottom_up = pitch < 0;
        let h_usize = h as usize;
        let (vw, vh) = visible_size(w, h, self.visible_width, self.visible_height);

        match self.output_format {
            OutputFormat::Rgb32 => {
                // Only the visible rows are read: scanline0 is the picture's top row
                // in both layouts, and the padding sits below the picture.
                let (w, h, h_usize) = (vw, vh, vh as usize);
                let row_bytes = (w as usize).checked_mul(4).ok_or("RGB32 width overflow")?;
                if stride < row_bytes {
                    return Err("RGB32 stride smaller than width*4".into());
                }
                let span = stride.checked_mul(h_usize).ok_or("RGB32 span overflow")?;
                let start_off = if bottom_up {
                    let top_row = scanline0
                        .checked_sub(stride.checked_mul(h_usize.saturating_sub(1)).ok_or("bottom-up overflow")?)
                        .ok_or("bottom-up pointer underflow")?;
                    top_row.checked_sub(buf_start).ok_or("bottom-up slice out of bounds")?
                } else {
                    scanline0.checked_sub(buf_start).ok_or("scanline0 before buffer start")?
                };
                let end_off = start_off.checked_add(span).ok_or("RGB32 end overflow")?;
                let pixels = full.get(start_off..end_off).ok_or("RGB32 buffer shorter than frame")?;
                scale::bgra_to_scaled_jpeg_oriented(pixels, w, h, stride, bottom_up, self.rotation, target_width)
            }
            OutputFormat::Nv12 | OutputFormat::P010 => {
                if bottom_up {
                    return Err("bottom-up YUV surfaces are not supported".into());
                }
                let start_off = scanline0.checked_sub(buf_start).ok_or("YUV scanline0 before buffer start")?;
                let avail = full.get(start_off..).ok_or("YUV start offset out of bounds")?;
                let uv_rows = h_usize.div_ceil(2);
                // Hardware D3D11/DXVA decoders often align surface height to 16 rows
                // (e.g. 1088 for 1080p); derive the real plane height from `avail.len()`.
                let surface_rows = if stride > 0 {
                    let est = (avail.len() * 2) / (3 * stride);
                    if est >= h_usize { est } else { h_usize }
                } else {
                    return Err("zero YUV stride".into());
                };
                let y_plane_bytes = stride.checked_mul(surface_rows).ok_or("YUV Y plane overflow")?;
                let uv_plane_bytes = stride.checked_mul(uv_rows).ok_or("YUV UV plane overflow")?;
                let total_need = y_plane_bytes.checked_add(uv_plane_bytes).ok_or("YUV total overflow")?;
                if avail.len() < total_need {
                    return Err("truncated NV12/P010 buffer".into());
                }
                let y_plane = &avail[..y_plane_bytes];
                let uv_plane = &avail[y_plane_bytes..total_need];
                if self.output_format == OutputFormat::P010 {
                    scale::p010_to_scaled_jpeg(y_plane, uv_plane, vw, vh, stride, stride, self.rotation, target_width)
                } else {
                    scale::nv12_to_scaled_jpeg(y_plane, uv_plane, vw, vh, stride, stride, self.rotation, target_width)
                }
            }
        }
    }
}

/// The picture inside a decoded frame of `w`×`h`: the container's size when
/// it fits, otherwise the whole frame.
fn visible_size(w: u32, h: u32, visible_w: u32, visible_h: u32) -> (u32, u32) {
    let pick = |frame: u32, visible: u32| if visible > 0 && visible <= frame { visible } else { frame };
    (pick(w, visible_w), pick(h, visible_h))
}

impl Drop for NativeDecoder {
    fn drop(&mut self) {
        shutdown_mft(&self.transform);
    }
}

fn shutdown_mft(transform: &IMFTransform) {
    if let Ok(sd) = transform.cast::<IMFShutdown>() {
        // SAFETY: Shutting down an MFT we own before releasing it.
        let _ = unsafe { sd.Shutdown() };
    }
}

struct OutputConfig {
    format: OutputFormat,
    width: u32,
    height: u32,
    stride: i32,
}

fn negotiate_types(transform: &IMFTransform, subtype: GUID, width: u32, height: u32) -> Result<OutputConfig, String> {
    // SAFETY: Valid COM calls on `transform`.
    let in_type: IMFMediaType = unsafe { MFCreateMediaType() }.map_err(|e| e.to_string())?;
    unsafe {
        in_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|e| e.to_string())?;
        in_type.SetGUID(&MF_MT_SUBTYPE, &subtype).map_err(|e| e.to_string())?;
        let packed_size = ((width as u64) << 32) | (height as u64);
        in_type.SetUINT64(&MF_MT_FRAME_SIZE, packed_size).map_err(|e| e.to_string())?;
        transform.SetInputType(0, &in_type, 0).map_err(|e| format!("SetInputType failed: {e}"))?;
    }
    let cfg = configure_output_type(transform, width, height)?;
    unsafe {
        let _ = transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0);
        let _ = transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0);
    }
    Ok(cfg)
}

fn create_d3d11_manager(transform: &IMFTransform) -> (Option<ID3D11Device>, Option<IMFDXGIDeviceManager>) {
    let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
    let mut device: Option<ID3D11Device> = None;
    let flags = D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT;
    // SAFETY: Creates an owned D3D11 hardware device with valid output pointers.
    if unsafe {
        D3D11CreateDevice(
            None::<&IDXGIAdapter>,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            flags,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
    }
    .is_err()
    {
        return (None, None);
    }
    let Some(dev) = device else { return (None, None) };
    if let Ok(mt) = dev.cast::<ID3D11Multithread>() {
        // SAFETY: Enables D3D11 multithread protection required by Media Foundation.
        let _ = unsafe { mt.SetMultithreadProtected(true) };
    }
    let mut token = 0u32;
    let mut mgr: Option<IMFDXGIDeviceManager> = None;
    // SAFETY: Output locals are valid; returned COM object is owned by `mgr`.
    if unsafe { MFCreateDXGIDeviceManager(&mut token, &mut mgr) }.is_err() {
        return (None, None);
    }
    let Some(mgr) = mgr else { return (None, None) };
    // SAFETY: Associates our D3D11 device with `mgr` using the token from creation.
    if unsafe { mgr.ResetDevice(&dev, token) }.is_err() {
        return (None, None);
    }
    let raw = mgr.as_raw() as usize;
    // SAFETY: `mgr` stays alive inside `NativeDecoder` while `transform` uses it.
    if unsafe { transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, raw) }.is_err() {
        return (None, None);
    }
    (Some(dev), Some(mgr))
}

fn configure_output_type(transform: &IMFTransform, fallback_w: u32, fallback_h: u32) -> Result<OutputConfig, String> {
    let mut p010_fallback: Option<IMFMediaType> = None;
    let mut rgb_fallback: Option<IMFMediaType> = None;
    for i in 0..32u32 {
        // SAFETY: Enumerates MFT output media types by index.
        let Ok(mt) = (unsafe { transform.GetOutputAvailableType(0, i) }) else { break };
        let Ok(sub) = (unsafe { mt.GetGUID(&MF_MT_SUBTYPE) }) else { continue };
        if sub == MFVideoFormat_NV12 {
            // SAFETY: Sets the negotiated NV12 output media type.
            unsafe { transform.SetOutputType(0, &mt, 0) }.map_err(|e| format!("SetOutputType(NV12) failed: {e}"))?;
            return Ok(extract_output_config(&mt, OutputFormat::Nv12, fallback_w, fallback_h));
        }
        if sub == MFVideoFormat_P010 && p010_fallback.is_none() {
            p010_fallback = Some(mt);
        } else if sub == MFVideoFormat_RGB32 && rgb_fallback.is_none() {
            rgb_fallback = Some(mt);
        }
    }
    if let Some(mt) = p010_fallback {
        // SAFETY: Sets the negotiated P010 output media type (10-bit HEVC/AV1).
        unsafe { transform.SetOutputType(0, &mt, 0) }.map_err(|e| format!("SetOutputType(P010) failed: {e}"))?;
        return Ok(extract_output_config(&mt, OutputFormat::P010, fallback_w, fallback_h));
    }
    if let Some(mt) = rgb_fallback {
        // SAFETY: Sets the negotiated RGB32 output media type.
        unsafe { transform.SetOutputType(0, &mt, 0) }.map_err(|e| format!("SetOutputType(RGB32) failed: {e}"))?;
        return Ok(extract_output_config(&mt, OutputFormat::Rgb32, fallback_w, fallback_h));
    }
    Err("no NV12, P010, or RGB32 output type supported by Media Foundation MFT".into())
}

fn extract_output_config(mt: &IMFMediaType, format: OutputFormat, fallback_w: u32, fallback_h: u32) -> OutputConfig {
    // SAFETY: Reading attributes from a valid `IMFMediaType`.
    let (width, height) = unsafe { mt.GetUINT64(&MF_MT_FRAME_SIZE) }
        .ok()
        .map(|v| ((v >> 32) as u32, (v & 0xFFFF_FFFF) as u32))
        .filter(|&(w, h)| w > 0 && h > 0)
        .unwrap_or((fallback_w, fallback_h));
    let min_stride = match format {
        OutputFormat::Nv12 => width as i32,
        OutputFormat::P010 => (width as i32).saturating_mul(2),
        OutputFormat::Rgb32 => (width as i32).saturating_mul(4),
    };
    let stride = unsafe { mt.GetUINT32(&MF_MT_DEFAULT_STRIDE) }
        .ok()
        .map(|s| s as i32)
        .filter(|&s| s.unsigned_abs() as i32 >= min_stride)
        .unwrap_or(min_stride);
    OutputConfig { format, width, height, stride }
}

#[cfg(test)]
mod tests {
    use super::visible_size;

    #[test]
    fn padded_frames_are_cropped_to_the_container_size() {
        assert_eq!(visible_size(96, 64, 96, 54), (96, 54));
        assert_eq!(visible_size(640, 368, 640, 360), (640, 360));
        assert_eq!(visible_size(1920, 1088, 1920, 1080), (1920, 1080));
        // Unknown or larger container sizes keep the decoded frame.
        assert_eq!(visible_size(1280, 720, 0, 0), (1280, 720));
        assert_eq!(visible_size(1280, 720, 1920, 1080), (1280, 720));
    }
}
