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

/// The playlist item that opens `capture`.
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
    if !matches!(capture.kind, CaptureKind::Microphone) {
        if let Some(audio) = &capture.audio {
            options.push(format!(":input-slave=avaudiocapture://{audio}"));
        }
    }
    Item { uri, title: title.to_string(), options }
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

#[cfg(not(target_os = "macos"))]
pub fn devices() -> Devices {
    Devices::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_items() {
        let cam = item(&Capture { kind: CaptureKind::Camera, device: "abc".into(), audio: Some("mic".into()), fps: None });
        assert_eq!(cam.uri, "avcapture://abc");
        assert!(cam.options.contains(&":input-slave=avaudiocapture://mic".to_string()));
        let screen = item(&Capture { kind: CaptureKind::Screen, device: "1".into(), audio: None, fps: Some(15) });
        assert_eq!(screen.uri, "screen://");
        assert!(screen.options.contains(&":screen-fps=15".to_string()));
        assert!(screen.options.contains(&":screen-display-id=1".to_string()));
    }
}
