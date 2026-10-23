//! Audio and video effects the user sets while playing: equaliser, stereo
//! mode, colour adjustments, logo and marquee overlays, and zoom. They
//! belong to the player rather than to a file, so they stay on when the
//! next item starts (like VLC's Effects and Filters window), until ReVLC quits.

use serde::{Deserialize, Serialize};

/// libVLC's equaliser limits, in dB.
pub const EQ_MIN: f32 = -20.0;
pub const EQ_MAX: f32 = 20.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Equalizer {
    pub enabled: bool,
    /// The preset the values came from; `None` once a band is moved by hand.
    pub preset: Option<u32>,
    /// dB, -20 to 20.
    pub preamp: f32,
    /// dB per band, -20 to 20 (libVLC 3 has 10 bands).
    pub bands: Vec<f32>,
}

impl Default for Equalizer {
    fn default() -> Self {
        // VLC's equaliser starts on its first preset, "Flat".
        Self { enabled: false, preset: Some(0), preamp: 0.0, bands: vec![0.0; 10] }
    }
}

impl Equalizer {
    /// Limits the values to what libVLC accepts and gives it exactly
    /// `band_count` bands. Values that aren't numbers become 0.
    pub fn normalized(mut self, band_count: usize) -> Self {
        let clamp = |v: f32| if v.is_finite() { v.clamp(EQ_MIN, EQ_MAX) } else { 0.0 };
        self.preamp = clamp(self.preamp);
        self.bands.resize(band_count, 0.0);
        for b in &mut self.bands {
            *b = clamp(*b);
        }
        self
    }

    /// Whether these values are what `preset` holds, so the UI can keep
    /// naming it. Anything else is a custom setting.
    pub fn matches(&self, preamp: f32, bands: &[f32]) -> bool {
        let close = |a: f32, b: f32| (a - b).abs() < 0.05;
        close(self.preamp, preamp) && self.bands.len() == bands.len() && self.bands.iter().zip(bands).all(|(&a, &b)| close(a, b))
    }
}

/// Stereo mode (VLC's Audio → Stereo Mode), as libVLC's stereo-mode codes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StereoMode {
    #[default]
    Stereo,
    Reverse,
    Left,
    Right,
    /// Dolby Surround decoding.
    Dolby,
    Mono,
}

impl StereoMode {
    pub fn code(self) -> i32 {
        match self {
            Self::Stereo => 1,
            Self::Reverse => 2,
            Self::Left => 3,
            Self::Right => 4,
            Self::Dolby => 5,
            // Not in libvlc_audio_output_channel_t, but libVLC 3's audio
            // output has it (AOUT_VAR_CHAN_MONO) and accepts it.
            Self::Mono => 7,
        }
    }
}

/// Colour adjustments, in the ranges of VLC's "adjust" filter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Adjust {
    pub enabled: bool,
    /// 0–2, 1 unchanged.
    pub brightness: f32,
    /// 0–2, 1 unchanged.
    pub contrast: f32,
    /// Degrees, -180–180.
    pub hue: f32,
    /// 0–3, 1 unchanged.
    pub saturation: f32,
    /// 0.01–10, 1 unchanged.
    pub gamma: f32,
}

impl Default for Adjust {
    fn default() -> Self {
        Self { enabled: false, brightness: 1.0, contrast: 1.0, hue: 0.0, saturation: 1.0, gamma: 1.0 }
    }
}

impl Adjust {
    pub fn normalized(self) -> Self {
        let d = Self::default();
        let clamp = |v: f32, lo: f32, hi: f32, default: f32| if v.is_finite() { v.clamp(lo, hi) } else { default };
        Self {
            enabled: self.enabled,
            brightness: clamp(self.brightness, 0.0, 2.0, d.brightness),
            contrast: clamp(self.contrast, 0.0, 2.0, d.contrast),
            hue: clamp(self.hue, -180.0, 180.0, d.hue),
            saturation: clamp(self.saturation, 0.0, 3.0, d.saturation),
            gamma: clamp(self.gamma, 0.01, 10.0, d.gamma),
        }
    }
}

/// VLC's overlay positions: 0 centre, 1 left, 2 right, 4 top, 8 bottom,
/// and sums of them for the corners.
pub const POSITIONS: [i32; 9] = [0, 1, 2, 4, 5, 6, 8, 9, 10];

fn position(code: i32, default: i32) -> i32 {
    if POSITIONS.contains(&code) { code } else { default }
}

/// An image shown over the video (VLC's logo filter).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Logo {
    pub enabled: bool,
    /// A PNG or other image file.
    pub file: String,
    pub position: i32,
    /// 0–255.
    pub opacity: i32,
}

impl Default for Logo {
    fn default() -> Self {
        Self { enabled: false, file: String::new(), position: 6, opacity: 255 }
    }
}

impl Logo {
    pub fn normalized(self) -> Self {
        Self { position: position(self.position, 6), opacity: self.opacity.clamp(0, 255), ..self }
    }
}

/// Text shown over the video (VLC's marquee filter). The text can use
/// VLC's time and metadata codes, like `%H:%M` or `$t`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Marquee {
    pub enabled: bool,
    pub text: String,
    pub position: i32,
    /// Font size in pixels; 0 lets VLC pick.
    pub size: i32,
    /// 0xRRGGBB.
    pub color: i32,
    /// 0–255.
    pub opacity: i32,
    /// How long it shows, in ms; 0 means for good.
    pub timeout: i32,
}

impl Default for Marquee {
    fn default() -> Self {
        Self { enabled: false, text: String::new(), position: 5, size: 0, color: 0xFFFFFF, opacity: 255, timeout: 0 }
    }
}

impl Marquee {
    pub fn normalized(self) -> Self {
        Self {
            position: position(self.position, 5),
            size: self.size.clamp(0, 400),
            color: self.color.clamp(0, 0xFFFFFF),
            opacity: self.opacity.clamp(0, 255),
            timeout: self.timeout.max(0),
            ..self
        }
    }
}

/// VLC's zoom presets (Video → Zoom); 0 fits the video to the window.
pub const ZOOMS: [f32; 5] = [0.0, 0.25, 0.5, 1.0, 2.0];

/// The nearest preset to `zoom`, so any value from the UI is one VLC offers.
pub fn zoom_preset(zoom: f32) -> f32 {
    if !zoom.is_finite() || zoom <= 0.0 {
        return 0.0;
    }
    ZOOMS[1..].iter().copied().min_by(|a, b| (a - zoom).abs().total_cmp(&(b - zoom).abs())).unwrap_or(0.0)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Effects {
    pub equalizer: Equalizer,
    pub stereo: StereoMode,
    pub adjust: Adjust,
    pub logo: Logo,
    pub marquee: Marquee,
    /// 0 fits the window; otherwise one of [`ZOOMS`].
    pub zoom: f32,
    /// Show teletext pages with a see-through background.
    pub teletext_transparent: bool,
}

impl Default for Effects {
    fn default() -> Self {
        Self {
            equalizer: Equalizer::default(),
            stereo: StereoMode::default(),
            adjust: Adjust::default(),
            logo: Logo::default(),
            marquee: Marquee::default(),
            zoom: 0.0,
            // VLC's default (vbi-opaque off).
            teletext_transparent: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equalizer_is_clamped_and_sized() {
        let eq = Equalizer { enabled: true, preset: None, preamp: 30.0, bands: vec![-25.0, 3.0, f32::NAN] }.normalized(10);
        assert_eq!(eq.preamp, 20.0);
        assert_eq!(eq.bands.len(), 10);
        assert_eq!(&eq.bands[..4], &[-20.0, 3.0, 0.0, 0.0]);
        let long = Equalizer { bands: vec![1.0; 12], ..Default::default() }.normalized(10);
        assert_eq!(long.bands, vec![1.0; 10]);
    }

    #[test]
    fn equalizer_matches_its_preset() {
        let preset = [3.0, 2.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let eq = Equalizer { enabled: true, preset: Some(4), preamp: 6.0, bands: preset.to_vec() };
        assert!(eq.matches(6.0, &preset));
        assert!(eq.matches(6.01, &preset));
        assert!(!eq.matches(5.0, &preset));
        let mut moved = eq.clone();
        moved.bands[2] = 4.0;
        assert!(!moved.matches(6.0, &preset));
    }

    #[test]
    fn stereo_modes_use_vlc_codes() {
        let modes = [StereoMode::Stereo, StereoMode::Reverse, StereoMode::Left, StereoMode::Right, StereoMode::Dolby, StereoMode::Mono];
        assert_eq!(modes.map(StereoMode::code), [1, 2, 3, 4, 5, 7]);
        assert_eq!(serde_json::to_string(&StereoMode::Dolby).unwrap(), "\"dolby\"");
    }

    #[test]
    fn adjust_is_clamped() {
        let a = Adjust { enabled: true, brightness: 5.0, contrast: -1.0, hue: 400.0, saturation: f32::INFINITY, gamma: 0.0 }.normalized();
        assert_eq!((a.brightness, a.contrast, a.hue, a.saturation, a.gamma), (2.0, 0.0, 180.0, 1.0, 0.01));
        assert!(a.enabled);
    }

    #[test]
    fn overlays_keep_valid_positions() {
        assert_eq!(Logo { position: 3, opacity: 300, ..Default::default() }.normalized(), Logo { opacity: 255, ..Default::default() });
        assert_eq!(Logo { position: 9, ..Default::default() }.normalized().position, 9);
        let m = Marquee { position: 7, size: -4, color: 0x1000000, timeout: -1, ..Default::default() }.normalized();
        assert_eq!((m.position, m.size, m.color, m.timeout), (5, 0, 0xFFFFFF, 0));
    }

    #[test]
    fn zoom_snaps_to_vlc_presets() {
        assert_eq!(zoom_preset(0.0), 0.0);
        assert_eq!(zoom_preset(-1.0), 0.0);
        assert_eq!(zoom_preset(f32::NAN), 0.0);
        assert_eq!(zoom_preset(0.3), 0.25);
        assert_eq!(zoom_preset(1.0), 1.0);
        assert_eq!(zoom_preset(1.6), 2.0);
        assert_eq!(zoom_preset(10.0), 2.0);
    }
}
