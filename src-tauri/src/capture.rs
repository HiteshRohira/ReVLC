//! Capture devices: cameras, microphones and screens, opened through
//! libVLC's AVFoundation and screen capture modules.

use serde::{Deserialize, Serialize};

use crate::playlist::Item;

#[derive(Clone, Debug, Serialize)]
pub struct Device {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Devices {
    pub video: Vec<Device>,
    pub audio: Vec<Device>,
    pub screens: Vec<Device>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureKind {
    Camera,
    Microphone,
    Screen,
}

/// The capture request from the UI.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    pub kind: CaptureKind,
    /// Camera, microphone or screen id ("" for the default).
    pub device: String,
    /// Audio to record alongside a camera or screen, if any.
    pub audio: Option<String>,
    /// Frames per second for screen capture.
    pub fps: Option<u32>,
}

/// The playlist item that opens `capture`. On Windows, cameras and
/// microphones go through DirectShow, and a screen id is its "x,y,w,h".
#[cfg(windows)]
pub fn item(capture: &Capture) -> Item {
    let mut options = vec![":live-caching=300".to_string()];
    let audio = capture.audio.as_deref().unwrap_or("none");
    let (uri, title) = match capture.kind {
        CaptureKind::Camera => {
            options.push(format!(":dshow-vdev={}", capture.device));
            options.push(format!(":dshow-adev={audio}"));
            ("dshow://".to_string(), "Camera")
        }
        CaptureKind::Microphone => {
            options.push(":dshow-vdev=none".to_string());
            options.push(format!(":dshow-adev={}", capture.device));
            ("dshow://".to_string(), "Microphone")
        }
        CaptureKind::Screen => {
            options.push(format!(":screen-fps={}", capture.fps.unwrap_or(30).clamp(1, 60)));
            let rect: Vec<&str> = capture.device.split(',').collect();
            if let [x, y, w, h] = rect[..] {
                options.extend([format!(":screen-left={x}"), format!(":screen-top={y}"), format!(":screen-width={w}"), format!(":screen-height={h}")]);
            }
            if capture.audio.is_some() {
                options.extend([":input-slave=dshow://".to_string(), ":dshow-vdev=none".to_string(), format!(":dshow-adev={audio}")]);
            }
            ("screen://".to_string(), "Screen")
        }
    };
    Item { uri, title: title.to_string(), options, trusted: true }
}

/// The playlist item that opens `capture`.
#[cfg(not(any(windows, target_os = "linux")))]
pub fn item(capture: &Capture) -> Item {
    let mut options = vec![":live-caching=300".to_string()];
    let (uri, title) = match capture.kind {
        CaptureKind::Camera => (format!("avcapture://{}", capture.device), "Camera"),
        CaptureKind::Microphone => (format!("avaudiocapture://{}", capture.device), "Microphone"),
        CaptureKind::Screen => {
            options.push(format!(":screen-fps={}", capture.fps.unwrap_or(30).clamp(1, 60)));
            if !capture.device.is_empty() {
                options.push(format!(":screen-display-id={}", capture.device));
            }
            ("screen://".to_string(), "Screen")
        }
    };
    if !matches!(capture.kind, CaptureKind::Microphone)
        && let Some(audio) = &capture.audio
    {
        options.push(format!(":input-slave=avaudiocapture://{audio}"));
    }
    Item { uri, title: title.to_string(), options, trusted: true }
}

#[cfg(target_os = "linux")]
pub fn item(capture: &Capture) -> Item {
    let mut options = vec![":live-caching=300".to_string()];
    let (uri, title) = match capture.kind {
        CaptureKind::Camera => (format!("v4l2://{}", capture.device), "Camera"),
        CaptureKind::Microphone => (format!("pulse://{}", capture.device), "Microphone"),
        CaptureKind::Screen => {
            options.push(format!(":screen-fps={}", capture.fps.unwrap_or(30).clamp(1, 60)));
            ("screen://".to_string(), "Screen")
        }
    };
    if !matches!(capture.kind, CaptureKind::Microphone)
        && let Some(audio) = &capture.audio
    {
        options.push(format!(":input-slave=pulse://{audio}"));
    }
    Item { uri, title: title.to_string(), options, trusted: true }
}

/// Lists cameras, microphones and screens. Must run on the main thread.
#[cfg(target_os = "macos")]
pub fn devices() -> Devices {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{MainThreadMarker, msg_send};
    use objc2_app_kit::NSScreen;
    use objc2_foundation::{NSArray, NSNumber, NSString, ns_string};

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {}

    fn av_devices(media_type: &NSString) -> Vec<Device> {
        let Some(class) = AnyClass::get(c"AVCaptureDevice") else { return Vec::new() };
        // SAFETY: +[AVCaptureDevice devicesWithMediaType:] returns an array of devices,
        // each with NSString uniqueID and localizedName.
        unsafe {
            let list: Option<Retained<NSArray<AnyObject>>> = msg_send![class, devicesWithMediaType: media_type];
            list.map(|list| {
                list.iter()
                    .map(|d| {
                        let id: Retained<NSString> = msg_send![&*d, uniqueID];
                        let name: Retained<NSString> = msg_send![&*d, localizedName];
                        Device { id: id.to_string(), name: name.to_string() }
                    })
                    .collect()
            })
            .unwrap_or_default()
        }
    }

    // AVMediaTypeVideo, AVMediaTypeMuxed and AVMediaTypeAudio.
    let mut video = av_devices(ns_string!("vide"));
    video.extend(av_devices(ns_string!("muxx")));
    let audio = av_devices(ns_string!("soun"));
    let screens = MainThreadMarker::new()
        .map(|mtm| {
            NSScreen::screens(mtm)
                .iter()
                .enumerate()
                .map(|(i, screen)| {
                    let number = screen.deviceDescription().objectForKey(ns_string!("NSScreenNumber"));
                    let id = number
                        .and_then(|n| n.downcast::<NSNumber>().ok())
                        .map(|n| n.unsignedIntValue().to_string())
                        .unwrap_or_default();
                    let name = screen.localizedName().to_string();
                    Device { id, name: if name.is_empty() { format!("Display {}", i + 1) } else { name } }
                })
                .collect()
        })
        .unwrap_or_default();
    Devices { video, audio, screens }
}

/// Lists DirectShow cameras and microphones (by the names libVLC's dshow
/// module matches on) and the displays.
#[cfg(windows)]
pub fn devices() -> Devices {
    use windows::Win32::Media::DirectShow::ICreateDevEnum;
    use windows::Win32::System::Com::StructuredStorage::IPropertyBag;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IMoniker};
    use windows::Win32::System::Variant::{VARIANT, VariantClear};
    use windows::core::{GUID, w};
    use windows_sys::Win32::Foundation::{LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};

    const SYSTEM_DEVICE_ENUM: GUID = GUID::from_u128(0x62be5d10_60eb_11d0_bd3b_00a0c911ce86);
    const VIDEO_INPUT: GUID = GUID::from_u128(0x860bb310_5d01_11d0_bd3b_00a0c911ce86);
    const AUDIO_INPUT: GUID = GUID::from_u128(0x33d9a762_90c8_11d0_bd43_00a0c911ce86);

    fn dshow(category: &GUID) -> Vec<Device> {
        let mut out = Vec::new();
        unsafe {
            let Ok(devenum) = CoCreateInstance::<_, ICreateDevEnum>(&SYSTEM_DEVICE_ENUM, None, CLSCTX_INPROC_SERVER) else {
                return out;
            };
            let mut monikers = None;
            // S_FALSE (no devices) leaves the enumerator empty.
            if devenum.CreateClassEnumerator(category, &mut monikers, 0).is_err() {
                return out;
            }
            let Some(monikers) = monikers else { return out };
            loop {
                let mut moniker: [Option<IMoniker>; 1] = [None];
                if monikers.Next(&mut moniker, None).is_err() {
                    break;
                }
                let Some(moniker) = moniker[0].take() else { break };
                let Ok(bag) = moniker.BindToStorage::<_, _, IPropertyBag>(None, None) else { continue };
                let mut value = VARIANT::default();
                if bag.Read(w!("FriendlyName"), &mut value, None).is_ok() {
                    let name = value.Anonymous.Anonymous.Anonymous.bstrVal.to_string();
                    if !name.is_empty() {
                        out.push(Device { id: name.clone(), name });
                    }
                }
                let _ = VariantClear(&mut value);
            }
        }
        out
    }

    unsafe extern "system" fn add_screen(_: HMONITOR, _: HDC, rect: *mut RECT, screens: LPARAM) -> i32 {
        unsafe {
            let r = &*rect;
            let screens = &mut *(screens as *mut Vec<Device>);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            screens.push(Device { id: format!("{},{},{w},{h}", r.left, r.top), name: format!("Display {} ({w}×{h})", screens.len() + 1) });
        }
        1
    }

    // Commands run on the main thread, where COM is already initialised
    // as single-threaded; this is a no-op there.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let mut screens: Vec<Device> = Vec::new();
    unsafe {
        EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(add_screen), &mut screens as *mut Vec<Device> as LPARAM);
    }
    Devices { video: dshow(&VIDEO_INPUT), audio: dshow(&AUDIO_INPUT), screens }
}

#[cfg(target_os = "linux")]
pub fn devices() -> Devices {
    let mut video: Vec<Device> = std::fs::read_dir("/sys/class/video4linux")
        .into_iter().flatten().flatten()
        .map(|entry| {
            let id = format!("/dev/{}", entry.file_name().to_string_lossy());
            let name = std::fs::read_to_string(entry.path().join("name")).unwrap_or_else(|_| id.clone()).trim().to_string();
            Device { id, name }
        }).collect();
    video.sort_by(|a, b| a.id.cmp(&b.id));
    Devices {
        video,
        audio: vec![Device { id: "default".into(), name: "Default microphone".into() }],
        screens: vec![Device { id: String::new(), name: "Desktop (X11)".into() }],
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
pub fn devices() -> Devices {
    Devices::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(any(windows, target_os = "linux")))]
    fn capture_items() {
        let cam = item(&Capture { kind: CaptureKind::Camera, device: "abc".into(), audio: Some("mic".into()), fps: None });
        assert_eq!(cam.uri, "avcapture://abc");
        assert!(cam.options.contains(&":input-slave=avaudiocapture://mic".to_string()));
        let screen = item(&Capture { kind: CaptureKind::Screen, device: "1".into(), audio: None, fps: Some(15) });
        assert_eq!(screen.uri, "screen://");
        assert!(screen.options.contains(&":screen-fps=15".to_string()));
        assert!(screen.options.contains(&":screen-display-id=1".to_string()));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn capture_items_linux() {
        let cam = item(&Capture { kind: CaptureKind::Camera, device: "/dev/video0".into(), audio: Some("default".into()), fps: None });
        assert_eq!(cam.uri, "v4l2:///dev/video0");
        assert!(cam.options.contains(&":input-slave=pulse://default".to_string()));
        let mic = item(&Capture { kind: CaptureKind::Microphone, device: "default".into(), audio: None, fps: None });
        assert_eq!(mic.uri, "pulse://default");
        let screen = item(&Capture { kind: CaptureKind::Screen, device: "".into(), audio: None, fps: Some(120) });
        assert_eq!(screen.uri, "screen://");
        assert!(screen.options.contains(&":screen-fps=60".to_string()));
        assert!(!screen.options.iter().any(|o| o.contains("display-id")));
    }

    #[test]
    #[cfg(windows)]
    fn capture_items() {
        let cam = item(&Capture { kind: CaptureKind::Camera, device: "USB Camera".into(), audio: None, fps: None });
        assert_eq!(cam.uri, "dshow://");
        assert!(cam.options.contains(&":dshow-vdev=USB Camera".to_string()));
        assert!(cam.options.contains(&":dshow-adev=none".to_string()));
        let screen = item(&Capture { kind: CaptureKind::Screen, device: "1920,0,1280,1024".into(), audio: None, fps: Some(15) });
        assert_eq!(screen.uri, "screen://");
        assert!(screen.options.contains(&":screen-left=1920".to_string()));
        assert!(screen.options.contains(&":screen-height=1024".to_string()));
    }
}
