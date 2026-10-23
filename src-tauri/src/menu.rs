//! The macOS menu bar. Item ids match the UI's action ids, so choosing a
//! menu item runs the same action as its keyboard shortcut.

use std::collections::HashMap;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Runtime};

/// (id, label) per menu; "-" is a separator.
const PLAYBACK: &[(&str, &str)] = &[
    ("playPause", "Play / Pause"),
    ("stop", "Stop"),
    ("next", "Next"),
    ("previous", "Previous"),
    ("-", ""),
    ("seekForward", "Jump Forward 5 s"),
    ("seekBack", "Jump Back 5 s"),
    ("seekForwardLong", "Jump Forward 30 s"),
    ("seekBackLong", "Jump Back 30 s"),
    ("nextFrame", "Next Frame"),
    ("jumpToTime", "Jump to Time…"),
    ("abLoop", "A–B Loop"),
    ("-", ""),
    ("nextChapter", "Next Chapter"),
    ("previousChapter", "Previous Chapter"),
    ("nextTitle", "Next Title"),
    ("previousTitle", "Previous Title"),
    ("discMenu", "Disc Menu"),
    ("-", ""),
    ("addBookmark", "Add Bookmark"),
    ("sleepTimer", "Sleep Timer"),
    ("-", ""),
    ("faster", "Faster"),
    ("slower", "Slower"),
    ("normalSpeed", "Normal Speed"),
];
const AUDIO: &[(&str, &str)] = &[
    ("volumeUp", "Increase Volume"),
    ("volumeDown", "Decrease Volume"),
    ("mute", "Mute"),
    ("-", ""),
    ("cycleAudio", "Next Audio Track"),
    ("cycleStereo", "Next Stereo Mode"),
    ("audioDelayUp", "Increase Audio Delay"),
    ("audioDelayDown", "Decrease Audio Delay"),
];
const VIDEO: &[(&str, &str)] = &[
    ("fullscreen", "Toggle Fullscreen"),
    ("snapshot", "Take Snapshot"),
    ("-", ""),
    ("cycleAspect", "Next Aspect Ratio"),
    ("cycleCrop", "Next Crop"),
    ("deinterlace", "Toggle Deinterlace"),
    ("-", ""),
    ("cycleVideo", "Next Video Track"),
    ("cycleZoom", "Zoom"),
    ("resetZoom", "Reset Zoom"),
];
const SUBTITLES: &[(&str, &str)] = &[
    ("toggleSubtitles", "Subtitles On / Off"),
    ("addSubtitle", "Add Subtitle File…"),
    ("-", ""),
    ("subtitleDelayUp", "Increase Subtitle Delay"),
    ("subtitleDelayDown", "Decrease Subtitle Delay"),
];

/// Converts a binding like "Shift+Cmd+O" to a menu accelerator. Only
/// bindings with ⌘ become accelerators, so plain keys (Space, arrows)
/// keep working in text fields.
fn accelerator(binding: &str) -> Option<String> {
    let parts: Vec<&str> = binding.split('+').collect();
    if !parts.contains(&"Cmd") {
        return None;
    }
    let key = *parts.last()?;
    let key = match key {
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        k => k,
    };
    let mut out: Vec<&str> = parts[..parts.len() - 1]
        .iter()
        .map(|m| match *m {
            "Cmd" => "CmdOrCtrl",
            "Alt" => "Alt",
            m => m,
        })
        .collect();
    out.push(key);
    Some(out.join("+"))
}

fn items<R: Runtime>(
    app: &AppHandle<R>,
    title: &str,
    entries: &[(&str, &str)],
    keys: &HashMap<String, String>,
) -> tauri::Result<Submenu<R>> {
    let menu = Submenu::new(app, title, true)?;
    for (id, label) in entries {
        if *id == "-" {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
        } else {
            let accel = keys.get(*id).and_then(|b| accelerator(b));
            menu.append(&MenuItem::with_id(app, *id, *label, true, accel.as_deref())?)?;
        }
    }
    Ok(menu)
}

/// Builds the menu bar. `keys` maps action ids to their current bindings.
pub fn build<R: Runtime>(app: &AppHandle<R>, keys: &HashMap<String, String>) -> tauri::Result<Menu<R>> {
    let sep = || PredefinedMenuItem::separator(app);
    let item = |id: &str, label: &str| {
        let accel = keys.get(id).and_then(|b| accelerator(b));
        MenuItem::with_id(app, id, label, true, accel.as_deref())
    };

    let app_menu = Submenu::with_items(
        app,
        "ReVLC",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("About ReVLC"), None)?,
            &MenuItem::with_id(app, "checkUpdates", "Check for Updates…", true, None::<&str>)?,
            &sep()?,
            &MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?,
            &sep()?,
            &PredefinedMenuItem::services(app, None)?,
            &sep()?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &sep()?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;
    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &item("open", "Open…")?,
            &item("openFolder", "Add Folder…")?,
            &item("openUrl", "Open Network Stream…")?,
            &item("openDisc", "Open Disc…")?,
            &item("openCapture", "Open Capture Device…")?,
            &sep()?,
            &item("savePlaylist", "Save Playlist…")?,
            &sep()?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &sep()?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, Some("Zoom"))?,
            &sep()?,
            &item("queue", "Up Next")?,
            &item("effects", "Effects and Filters…")?,
            &item("mediaInfo", "Media Information…")?,
            &item("codecInfo", "Codec Information…")?,
        ],
    )?;
    let help = Submenu::with_items(
        app,
        "Help",
        true,
        &[
            &item("help", "Keyboard Shortcuts")?,
            &MenuItem::with_id(app, "github", "ReVLC on GitHub", true, None::<&str>)?,
        ],
    )?;

    Menu::with_items(
        app,
        &[
            &app_menu,
            &file,
            &edit,
            &items(app, "Playback", PLAYBACK, keys)?,
            &items(app, "Audio", AUDIO, keys)?,
            &items(app, "Video", VIDEO, keys)?,
            &items(app, "Subtitles", SUBTITLES, keys)?,
            &window,
            &help,
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::accelerator;

    #[test]
    fn only_command_bindings_become_accelerators() {
        assert_eq!(accelerator("Cmd+O").as_deref(), Some("CmdOrCtrl+O"));
        assert_eq!(accelerator("Shift+Cmd+O").as_deref(), Some("Shift+CmdOrCtrl+O"));
        assert_eq!(accelerator("Cmd+ArrowRight").as_deref(), Some("CmdOrCtrl+Right"));
        assert_eq!(accelerator("Space"), None);
        assert_eq!(accelerator("Shift+S"), None);
    }
}
