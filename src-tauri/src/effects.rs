//! Audio and video effects the user sets while playing: equaliser, stereo
//! mode, audio filters, colour adjustments, rotation, video filters, logo
//! and marquee overlays, and zoom. They
//! belong to the player rather than to a file, so they stay on when the
//! next item starts (like VLC's Effects and Filters window), until ReVLC quits.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// libVLC's equaliser limits, in dB.
pub const EQ_MIN: f32 = -20.0;
pub const EQ_MAX: f32 = 20.0;
/// The preamp of libVLC's "Flat" preset, which leaves the level unchanged.
pub const FLAT_PREAMP: f32 = 12.0;

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
        // VLC's equaliser starts on its first preset, "Flat". Its filter
        // runs 12 dB down, so Flat's preamp of 12 dB leaves the level as is.
        Self { enabled: false, preset: Some(0), preamp: FLAT_PREAMP, bands: vec![0.0; 10] }
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

/// Rotating or flipping the picture (VLC's transform filter).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Transform {
    #[default]
    None,
    Rotate90,
    Rotate180,
    Rotate270,
    FlipHorizontal,
    FlipVertical,
    Transpose,
    AntiTranspose,
}

impl Transform {
    /// The transform filter's type, or `None` for no filter.
    pub fn vlc_type(self) -> Option<&'static str> {
        Some(match self {
            Self::None => return None,
            Self::Rotate90 => "90",
            Self::Rotate180 => "180",
            Self::Rotate270 => "270",
            Self::FlipHorizontal => "hflip",
            Self::FlipVertical => "vflip",
            Self::Transpose => "transpose",
            Self::AntiTranspose => "antitranspose",
        })
    }
}

/// A setting of a VLC filter. For video filters `key` is the filter's own
/// option name, given in the filter chain (`sharpen{sigma=0.5}`); for audio
/// filters it's the variable the filter reads from the audio output.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Param {
    pub key: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub default: f32,
    /// An integer option (VLC rejects a float for those).
    pub int: bool,
    /// Shown after the value, like "dB" or "ms".
    pub unit: &'static str,
}

/// A VLC filter the UI can turn on, with its settings.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct FilterSpec {
    /// Key in [`Effects::filters`].
    pub id: &'static str,
    pub label: &'static str,
    /// VLC's module name.
    pub module: &'static str,
    /// Options always given to a video filter, like "mode=edge".
    #[serde(skip)]
    pub fixed: &'static str,
    pub params: &'static [Param],
}

const fn param(key: &'static str, label: &'static str, min: f32, max: f32, step: f32, default: f32, unit: &'static str) -> Param {
    Param { key, label, min, max, step, default, int: false, unit }
}

const fn int(key: &'static str, label: &'static str, min: f32, max: f32, default: f32) -> Param {
    Param { key, label, min, max, step: 1.0, default, int: true, unit: "" }
}

const fn filter(id: &'static str, label: &'static str, module: &'static str, params: &'static [Param]) -> FilterSpec {
    FilterSpec { id, label, module, fixed: "", params }
}

/// VLC's video effects (Tools → Effects and Filters → Video Effects), with
/// VLC's defaults. Applied in this order, after the rotation.
pub const VIDEO_FILTERS: &[FilterSpec] = &[
    filter("rotate", "Rotate by angle", "rotate", &[param("angle", "Angle", 0.0, 359.0, 1.0, 30.0, "°")]),
    filter("sharpen", "Sharpen", "sharpen", &[param("sigma", "Strength", 0.0, 2.0, 0.01, 0.05, "")]),
    filter("denoise", "Denoise", "hqdn3d", &[param("luma-spat", "Strength", 0.0, 30.0, 0.5, 4.0, "")]),
    filter("gradfun", "Banding removal", "gradfun", &[param("strength", "Strength", 0.51, 20.0, 0.01, 1.2, "")]),
    filter("grain", "Film grain", "grain", &[param("variance", "Amount", 0.0, 10.0, 0.1, 2.0, "")]),
    filter("blur", "Blur", "gaussianblur", &[param("sigma", "Amount", 0.1, 10.0, 0.1, 2.0, "")]),
    filter("motionblur", "Motion blur", "motionblur", &[int("factor", "Amount", 1.0, 127.0, 80.0)]),
    filter("sepia", "Sepia", "sepia", &[int("intensity", "Intensity", 0.0, 255.0, 120.0)]),
    filter("posterize", "Posterize", "posterize", &[int("level", "Levels", 2.0, 32.0, 6.0)]),
    filter("invert", "Invert colours", "invert", &[]),
    FilterSpec { id: "edges", label: "Edge detection", module: "gradient", fixed: "mode=edge", params: &[] },
    filter("mirror", "Mirror", "mirror", &[]),
    filter("wave", "Waves", "wave", &[]),
    filter("ripple", "Water", "ripple", &[]),
    filter("psychedelic", "Psychedelic", "psychedelic", &[]),
    filter("anaglyph", "Anaglyph 3D", "anaglyph", &[]),
    filter("antiflicker", "Anti-flicker", "antiflicker", &[]),
];

/// VLC's audio effects besides the equaliser, with VLC's defaults. Applied
/// after the equaliser, in this order.
pub const AUDIO_FILTERS: &[FilterSpec] = &[
    filter("compressor", "Compressor", "compressor", &[
        param("compressor-rms-peak", "RMS / peak", 0.0, 1.0, 0.01, 0.2, ""),
        param("compressor-attack", "Attack", 1.5, 400.0, 0.5, 25.0, "ms"),
        param("compressor-release", "Release", 2.0, 800.0, 1.0, 100.0, "ms"),
        param("compressor-threshold", "Threshold", -30.0, 0.0, 0.5, -11.0, "dB"),
        param("compressor-ratio", "Ratio", 1.0, 20.0, 0.1, 4.0, ":1"),
        param("compressor-knee", "Knee", 1.0, 10.0, 0.1, 5.0, "dB"),
        param("compressor-makeup-gain", "Makeup gain", 0.0, 24.0, 0.5, 7.0, "dB"),
    ]),
    filter("spatializer", "Spatialiser", "spatializer", &[
        param("spatializer-roomsize", "Room size", 0.0, 1.1, 0.01, 0.85, ""),
        param("spatializer-width", "Width", 0.0, 1.0, 0.01, 1.0, ""),
        param("spatializer-wet", "Wet", 0.0, 1.0, 0.01, 0.4, ""),
        param("spatializer-dry", "Dry", 0.0, 1.0, 0.01, 0.5, ""),
        param("spatializer-damp", "Damping", 0.0, 1.0, 0.01, 0.5, ""),
    ]),
    filter("widener", "Stereo widener", "stereo_widen", &[
        param("stereowiden-delay", "Delay", 1.0, 100.0, 1.0, 20.0, "ms"),
        param("stereowiden-feedback", "Feedback", 0.0, 0.9, 0.01, 0.3, ""),
        param("stereowiden-crossfeed", "Crossfeed", 0.0, 0.8, 0.01, 0.3, ""),
        param("stereowiden-dry-mix", "Dry mix", 0.0, 1.0, 0.01, 0.8, ""),
    ]),
    filter("normalize", "Volume normalisation", "normvol", &[param("norm-max-level", "Maximum level", 0.1, 10.0, 0.1, 2.0, "")]),
    filter("pitch", "Pitch", "scaletempo_pitch", &[param("pitch-shift", "Shift", -12.0, 12.0, 0.5, 0.0, "semitones")]),
];

/// Whether a filter is on, and its settings (missing ones take the default).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filter {
    pub enabled: bool,
    pub values: BTreeMap<String, f32>,
}

fn spec(id: &str) -> Option<&'static FilterSpec> {
    VIDEO_FILTERS.iter().chain(AUDIO_FILTERS).find(|s| s.id == id)
}

impl Param {
    /// `value` within the range, rounded for integer options; the default
    /// for anything that isn't a number.
    pub fn clamp(&self, value: Option<f32>) -> f32 {
        let v = value.filter(|v| v.is_finite()).unwrap_or(self.default).clamp(self.min, self.max);
        if self.int { v.round() } else { v }
    }
}

impl Filter {
    fn value(&self, p: &Param) -> f32 {
        p.clamp(self.values.get(p.key).copied())
    }

    /// Keeps only `spec`'s settings, each within its range.
    pub fn normalized(self, spec: &FilterSpec) -> Self {
        let values = spec.params.iter().map(|p| (p.key.to_string(), self.value(p))).collect();
        Self { enabled: self.enabled, values }
    }
}

/// A number as VLC's option parser reads it: no exponent, trailing zeros trimmed.
fn number(v: f32, int: bool) -> String {
    if int {
        return format!("{}", v.round() as i64);
    }
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
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
    pub transform: Transform,
    /// Filters by [`FilterSpec::id`], video and audio.
    pub filters: BTreeMap<String, Filter>,
}

/// One audio filter setting, as the variable to set and its value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioParam {
    pub name: &'static str,
    pub value: f32,
    pub int: bool,
}

impl Effects {
    fn filter(&self, spec: &FilterSpec) -> Option<&Filter> {
        self.filters.get(spec.id).filter(|f| f.enabled)
    }

    /// Sets a filter, dropping unknown ones and settings out of range.
    /// Returns false for an unknown id.
    pub fn set_filter(&mut self, id: &str, filter: Filter) -> bool {
        let Some(spec) = spec(id) else { return false };
        self.filters.insert(id.to_string(), filter.normalized(spec));
        true
    }

    /// VLC's "video-filter" chain for these effects, like
    /// "transform{type=90}:adjust:sharpen{sigma=0.5}".
    pub fn video_chain(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(ty) = self.transform.vlc_type() {
            parts.push(format!("transform{{type={ty}}}"));
        }
        // libVLC's colour adjustments live in the same chain.
        if self.adjust.enabled {
            parts.push("adjust".into());
        }
        for spec in VIDEO_FILTERS {
            let Some(f) = self.filter(spec) else { continue };
            let mut options: Vec<String> = spec.params.iter().map(|p| format!("{}={}", p.key, number(f.value(p), p.int))).collect();
            if !spec.fixed.is_empty() {
                options.insert(0, spec.fixed.to_string());
            }
            parts.push(if options.is_empty() { spec.module.to_string() } else { format!("{}{{{}}}", spec.module, options.join(",")) });
        }
        parts.join(":")
    }

    /// VLC's "audio-filter" chain, the equaliser first.
    pub fn audio_chain(&self) -> String {
        let eq = self.equalizer.enabled.then_some("equalizer");
        let rest = AUDIO_FILTERS.iter().filter(|s| self.filter(s).is_some()).map(|s| s.module);
        eq.into_iter().chain(rest).collect::<Vec<_>>().join(":")
    }

    /// Every audio filter's settings (on or off), for the audio output.
    pub fn audio_params(&self) -> Vec<AudioParam> {
        AUDIO_FILTERS
            .iter()
            .flat_map(|spec| {
                let f = self.filters.get(spec.id).cloned().unwrap_or_default();
                spec.params.iter().map(move |p| AudioParam { name: p.key, value: f.value(p), int: p.int })
            })
            .collect()
    }
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
            transform: Transform::None,
            filters: BTreeMap::new(),
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
    fn default_equalizer_is_flat() {
        let eq = Equalizer::default();
        assert!(eq.matches(FLAT_PREAMP, &[0.0; 10]));
        assert_eq!(eq.preset, Some(0));
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

    fn on(values: &[(&str, f32)]) -> Filter {
        Filter { enabled: true, values: values.iter().map(|(k, v)| (k.to_string(), *v)).collect() }
    }

    #[test]
    fn video_chain_follows_the_effects() {
        let mut e = Effects::default();
        assert_eq!(e.video_chain(), "");
        e.transform = Transform::Rotate90;
        e.adjust.enabled = true;
        assert!(e.set_filter("sharpen", on(&[("sigma", 0.5)])));
        assert!(e.set_filter("invert", on(&[])));
        assert!(e.set_filter("edges", on(&[])));
        assert!(e.set_filter("posterize", on(&[("level", 5.6)])));
        e.set_filter("grain", Filter { enabled: false, ..on(&[("variance", 3.0)]) });
        assert_eq!(e.video_chain(), "transform{type=90}:adjust:sharpen{sigma=0.5}:posterize{level=6}:invert:gradient{mode=edge}");
        e.transform = Transform::FlipHorizontal;
        e.adjust.enabled = false;
        assert_eq!(e.video_chain(), "transform{type=hflip}:sharpen{sigma=0.5}:posterize{level=6}:invert:gradient{mode=edge}");
    }

    #[test]
    fn filter_settings_are_clamped_and_filled_in() {
        let mut e = Effects::default();
        assert!(!e.set_filter("nonsense", on(&[])));
        e.set_filter("sharpen", on(&[("sigma", 9.0), ("bogus", 1.0)]));
        assert_eq!(e.filters["sharpen"].values, BTreeMap::from([("sigma".to_string(), 2.0)]));
        e.set_filter("blur", on(&[("sigma", f32::NAN)]));
        assert_eq!(e.video_chain(), "sharpen{sigma=2}:gaussianblur{sigma=2}");
        e.set_filter("rotate", on(&[]));
        assert!(e.video_chain().starts_with("rotate{angle=30}:"));
    }

    #[test]
    fn audio_chain_keeps_the_equalizer_first() {
        let mut e = Effects::default();
        assert_eq!(e.audio_chain(), "");
        e.set_filter("pitch", on(&[("pitch-shift", 3.0)]));
        e.set_filter("compressor", on(&[]));
        assert_eq!(e.audio_chain(), "compressor:scaletempo_pitch");
        e.equalizer.enabled = true;
        assert_eq!(e.audio_chain(), "equalizer:compressor:scaletempo_pitch");
        let params = e.audio_params();
        assert_eq!(params.len(), AUDIO_FILTERS.iter().map(|s| s.params.len()).sum::<usize>());
        let get = |n: &str| params.iter().find(|p| p.name == n).unwrap().value;
        assert_eq!(get("pitch-shift"), 3.0);
        assert_eq!(get("compressor-ratio"), 4.0);
        assert_eq!(get("spatializer-roomsize"), 0.85);
    }

    #[test]
    fn numbers_read_like_vlc_expects() {
        assert_eq!(number(0.05, false), "0.05");
        assert_eq!(number(2.0, false), "2");
        assert_eq!(number(-0.0001, false), "0");
        assert_eq!(number(-11.5, false), "-11.5");
        assert_eq!(number(5.6, true), "6");
    }

    #[test]
    fn filter_ids_are_unique() {
        let mut ids: Vec<&str> = VIDEO_FILTERS.iter().chain(AUDIO_FILTERS).map(|s| s.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
        for p in VIDEO_FILTERS.iter().chain(AUDIO_FILTERS).flat_map(|s| s.params) {
            assert!(p.min <= p.default && p.default <= p.max, "{}", p.key);
        }
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
