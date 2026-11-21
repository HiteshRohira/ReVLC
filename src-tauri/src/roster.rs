//! Which player windows are open, which one has the focus, and when the
//! last one closing should quit the app. Pure bookkeeping, so it's tested
//! without a window system. The Settings window isn't a player window and
//! never comes through here.

/// The label of the first player window (the one in `tauri.conf.json`).
pub const MAIN: &str = "main";

#[derive(Default)]
pub struct Roster {
    /// Open player windows, oldest first.
    labels: Vec<String>,
    /// The player window that last had the focus.
    focused: Option<String>,
    /// The number the next new window's label gets.
    issued: u32,
}

impl Roster {
    /// Registers a new player window and returns its label: `main` for the
    /// first, then `player-2`, `player-3`… Labels are never reused, so a
    /// window still being torn down can't clash with a new one.
    pub fn open(&mut self) -> String {
        self.issued += 1;
        let label = if self.issued == 1 { MAIN.to_string() } else { format!("player-{}", self.issued) };
        self.labels.push(label.clone());
        label
    }

    /// Whether `label` names a player window (as opposed to Settings).
    pub fn contains(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l == label)
    }

    pub fn len(&self) -> usize {
        self.labels.len()
    }

    /// Notes that `label` has the focus (ignored for other windows).
    pub fn focus(&mut self, label: &str) {
        if self.contains(label) {
            self.focused = Some(label.to_string());
        }
    }

    /// Forgets a closed window. Returns whether it was the last one, which
    /// quits the app.
    pub fn close(&mut self, label: &str) -> bool {
        let before = self.labels.len();
        self.labels.retain(|l| l != label);
        if self.focused.as_deref() == Some(label) {
            self.focused = None;
        }
        before > 0 && self.labels.is_empty()
    }

    /// The window that menu actions and opened files go to: the one that
    /// last had the focus, else the newest.
    pub fn target(&self) -> Option<&str> {
        self.focused.as_deref().or_else(|| self.labels.last().map(String::as_str))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_unique_and_never_reused() {
        let mut w = Roster::default();
        assert_eq!(w.open(), "main");
        assert_eq!(w.open(), "player-2");
        w.close("player-2");
        assert_eq!(w.open(), "player-3");
        w.close("main");
        assert_eq!(w.open(), "player-4");
        assert_eq!(w.len(), 2);
        assert!(w.contains("player-3") && w.contains("player-4"));
    }

    #[test]
    fn quits_only_when_the_last_window_closes() {
        let mut w = Roster::default();
        w.open();
        let second = w.open();
        assert!(!w.close(&second));
        assert!(w.close("main"));
        // Closing something that isn't open (Settings) never quits.
        assert!(!w.close("settings"));
    }

    #[test]
    fn closing_a_stranger_with_windows_open_keeps_running() {
        let mut w = Roster::default();
        w.open();
        assert!(!w.close("settings"));
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn target_follows_focus_then_newest() {
        let mut w = Roster::default();
        assert_eq!(w.target(), None);
        w.open();
        let b = w.open();
        let c = w.open();
        assert_eq!(w.target(), Some(c.as_str()));
        w.focus("main");
        assert_eq!(w.target(), Some("main"));
        // Settings taking the focus doesn't change where actions go.
        w.focus("settings");
        assert_eq!(w.target(), Some("main"));
        // The focused window closing falls back to the newest.
        w.close("main");
        assert_eq!(w.target(), Some(c.as_str()));
        w.focus(&b);
        w.close(&b);
        assert_eq!(w.target(), Some(c.as_str()));
    }
}
