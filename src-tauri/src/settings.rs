//! User settings. Defaults match VLC 3's (from its vlcrc), except the
//! preferred subtitle language, which follows the system languages.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::vlc::ConfigValue;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Resume {
    /// VLC's macOS default.
    #[default]
    Ask,
    Always,
    Never,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    // General
    pub resume: Resume,
    /// VLC "play-and-stop": stop after each item instead of playing the next.
    pub stop_after_each: bool,
    pub pause_when_minimized: bool,
    pub seekbar_chapters: bool,
    /// Download and install new versions in the background.
    pub auto_update: bool,

    // Video
    pub video: bool,
    pub start_fullscreen: bool,
    pub always_on_top: bool,
    /// -1 automatic, 0 off, 1 on.
    pub deinterlace: i64,
    pub deinterlace_mode: String,
    /// Empty means VLC's default: the Desktop on macOS, Pictures on Windows.
    pub snapshot_path: String,
    pub snapshot_prefix: String,
    pub snapshot_format: String,
    pub snapshot_sequential: bool,

    // Audio
    pub audio: bool,
    /// Empty means the system default output.
    pub audio_device: String,
    pub audio_language: String,
    pub time_stretch: bool,
    pub replay_gain: String,
    pub spdif: bool,

    // Subtitles & OSD
    /// `None` means "use the system languages".
    pub subtitle_language: Option<String>,
    pub sub_autodetect: bool,
    pub sub_autodetect_fuzzy: i64,
    pub sub_encoding: String,
    pub font: String,
    /// VLC's relative size: 0 auto, 20 smaller, 18 small, 16 normal, 12 large, 6 larger.
    pub font_size: i64,
    pub font_color: i64,
    pub font_opacity: i64,
    pub bold: bool,
    /// 0 none, 2 thin, 4 normal, 6 thick.
    pub outline_thickness: i64,
    pub outline_color: i64,
    pub background_opacity: i64,
    pub background_color: i64,
    pub shadow_opacity: i64,
    pub sub_margin: i64,
    pub show_title: bool,
    pub title_timeout: i64,
    pub title_position: i64,

    // Input & codecs
    pub hardware_decoding: bool,
    pub file_caching: i64,
    pub network_caching: i64,
    pub skip_loop_filter: i64,

    /// Action id → key binding, only for bindings the user changed.
    pub shortcuts: BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            resume: Resume::Ask,
            stop_after_each: false,
            pause_when_minimized: false,
            seekbar_chapters: true,
            auto_update: true,

            video: true,
            start_fullscreen: false,
            always_on_top: false,
            deinterlace: -1,
            deinterlace_mode: "auto".into(),
            snapshot_path: String::new(),
            snapshot_prefix: "vlcsnap-".into(),
            snapshot_format: "png".into(),
            snapshot_sequential: false,

            audio: true,
            audio_device: String::new(),
            audio_language: String::new(),
            time_stretch: true,
            replay_gain: "none".into(),
            spdif: false,

            subtitle_language: None,
            sub_autodetect: true,
            sub_autodetect_fuzzy: 3,
            sub_encoding: String::new(),
            font: if cfg!(target_os = "macos") { "Helvetica Neue" } else { "Arial" }.into(),
            font_size: 0,
            font_color: 0xFFFFFF,
            font_opacity: 255,
            bold: false,
            outline_thickness: 4,
            outline_color: 0,
            background_opacity: 0,
            background_color: 0,
            shadow_opacity: 128,
            sub_margin: 0,
            show_title: false,
            title_timeout: 5000,
            title_position: 8,

            hardware_decoding: true,
            file_caching: 1000,
            network_caching: 1000,
            skip_loop_filter: 0,

            shortcuts: BTreeMap::new(),
        }
    }
}

impl Settings {
    pub fn load(data_dir: &Path) -> Self {
        std::fs::read(file(data_dir))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, data_dir: &Path) -> Result<(), String> {
        crate::store::write_json(&file(data_dir), self, true)
    }

    /// Where user snapshots go.
    pub fn snapshot_dir(&self, desktop: Option<PathBuf>) -> Option<PathBuf> {
        if self.snapshot_path.is_empty() { desktop } else { Some(PathBuf::from(&self.snapshot_path)) }
    }

    /// The VLC options these settings control, as (name, value) pairs.
    pub fn vlc_config(&self, system_languages: &[String]) -> Vec<(&'static str, ConfigValue)> {
        use ConfigValue::*;
        let sub_language = self.subtitle_language.clone().unwrap_or_else(|| system_languages.join(","));
        vec![
            ("play-and-stop", Bool(self.stop_after_each)),
            ("video", Bool(self.video)),
            ("deinterlace", Int(self.deinterlace)),
            ("deinterlace-mode", Str(self.deinterlace_mode.clone())),
            ("snapshot-prefix", Str(self.snapshot_prefix.clone())),
            ("snapshot-format", Str(self.snapshot_format.clone())),
            ("snapshot-sequential", Bool(self.snapshot_sequential)),
            ("audio", Bool(self.audio)),
            ("audio-language", Str(self.audio_language.clone())),
            ("audio-time-stretch", Bool(self.time_stretch)),
            ("audio-replay-gain-mode", Str(self.replay_gain.clone())),
            ("spdif", Bool(self.spdif)),
            ("sub-language", Str(sub_language)),
            ("sub-autodetect-file", Bool(self.sub_autodetect)),
            ("sub-autodetect-fuzzy", Int(self.sub_autodetect_fuzzy)),
            ("subsdec-encoding", Str(self.sub_encoding.clone())),
            ("freetype-font", Str(self.font.clone())),
            ("freetype-rel-fontsize", Int(self.font_size)),
            ("freetype-color", Int(self.font_color)),
            ("freetype-opacity", Int(self.font_opacity)),
            ("freetype-bold", Bool(self.bold)),
            ("freetype-outline-thickness", Int(self.outline_thickness)),
            ("freetype-outline-color", Int(self.outline_color)),
            ("freetype-background-opacity", Int(self.background_opacity)),
            ("freetype-background-color", Int(self.background_color)),
            ("freetype-shadow-opacity", Int(self.shadow_opacity)),
            ("sub-margin", Int(self.sub_margin)),
            ("video-title-show", Bool(self.show_title)),
            ("video-title-timeout", Int(self.title_timeout)),
            ("video-title-position", Int(self.title_position)),
            if cfg!(target_os = "macos") {
                ("videotoolbox", Bool(self.hardware_decoding))
            } else {
                ("avcodec-hw", Str(if self.hardware_decoding { "any" } else { "none" }.into()))
            },
            ("file-caching", Int(self.file_caching)),
            ("network-caching", Int(self.network_caching)),
            ("avcodec-skiploopfilter", Int(self.skip_loop_filter)),
        ]
    }
}

fn file(data_dir: &Path) -> PathBuf {
    data_dir.join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_vlc_defaults() {
        let s: Settings = serde_json::from_str(r#"{"fontSize": 12}"#).unwrap();
        assert_eq!(s.font_size, 12);
        assert_eq!(s.network_caching, 1000);
        assert_eq!(s.resume, Resume::Ask);
    }

    #[test]
    fn system_languages_unless_overridden() {
        let mut s = Settings::default();
        let langs = vec!["en".to_string(), "hi".to_string()];
        let lang = |s: &Settings| s.vlc_config(&langs).into_iter().find(|(k, _)| *k == "sub-language").unwrap().1;
        assert_eq!(lang(&s), ConfigValue::Str("en,hi".into()));
        s.subtitle_language = Some("fr".into());
        assert_eq!(lang(&s), ConfigValue::Str("fr".into()));
    }
}
