//! VLC-style command-line options. Files and URLs are opened; a few of
//! VLC's options are handled by ReVLC; other `--options` go to libVLC,
//! except ones that would take over or break ReVLC's window. See the README.

/// Options that would replace ReVLC's interface or video window, send the
/// video elsewhere, change where libVLC finds its plugins and settings, or
/// print something and quit.
const BLOCKED: &[&str] = &[
    "intf", "extraintf", "control", "vout", "video-wallpaper", "embedded-video", "daemon", "pidfile", "sout", "sout-all",
    "sout-keep", "vlm-conf", "config", "ignore-config", "reset-config", "reset-plugins-cache", "plugins-cache",
    "plugins-scan", "plugin-path", "data-path", "one-instance", "one-instance-when-started-from-file",
    "playlist-enqueue", "started-from-file", "help", "full-help", "longhelp", "version", "list", "list-verbose",
    "module", "save-config", "quiet",
];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cli {
    /// Files, folders and URLs to open.
    pub files: Vec<String>,
    pub fullscreen: bool,
    /// Per-media options for what's opened: `:start-time=…` and friends.
    pub media_options: Vec<String>,
    /// Quit once the queue has played to its end.
    pub play_and_exit: bool,
    /// Options passed on to libVLC when it starts.
    pub vlc_args: Vec<String>,
    /// Options left out: unsafe ones, and values that aren't valid.
    pub ignored: Vec<String>,
}

/// The option's name without dashes or a "no-" prefix, and its value.
fn split(arg: &str) -> (&str, Option<&str>) {
    let body = arg.trim_start_matches('-');
    let (name, value) = match body.split_once('=') {
        Some((n, v)) => (n, Some(v)),
        None => (body, None),
    };
    (name.strip_prefix("no-").unwrap_or(name), value)
}

/// Seconds like VLC takes them: a non-negative number.
fn seconds(value: &str) -> Option<f64> {
    value.trim().parse::<f64>().ok().filter(|s| s.is_finite() && *s >= 0.0)
}

fn is_number(arg: &str) -> bool {
    arg.parse::<f64>().is_ok()
}

/// Parses the arguments after the program name.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Cli {
    let mut cli = Cli::default();
    let mut args = args.into_iter().peekable();
    let mut only_files = false;
    while let Some(arg) = args.next() {
        if only_files || !arg.starts_with('-') || arg == "-" {
            if arg != "-" {
                cli.files.push(arg);
            }
            continue;
        }
        if arg == "--" {
            only_files = true;
            continue;
        }
        if !arg.starts_with("--") {
            match arg.as_str() {
                "-f" => cli.fullscreen = true,
                // macOS adds a process serial number when Finder starts an old-style app.
                a if a.starts_with("-psn_") => {}
                _ => cli.ignored.push(arg),
            }
            continue;
        }
        let negated = arg.starts_with("--no-");
        let (name, inline) = split(&arg);
        match name {
            "fullscreen" => cli.fullscreen = !negated,
            "play-and-exit" => cli.play_and_exit = !negated,
            "start-time" | "stop-time" | "run-time" => {
                // VLC takes "--start-time 30" as well as "--start-time=30".
                let separate = if inline.is_none() { args.next_if(|next| is_number(next)) } else { None };
                match inline.or(separate.as_deref()).and_then(seconds) {
                    Some(s) => cli.media_options.push(format!(":{name}={s}")),
                    None => cli.ignored.push(separate.map_or(arg.clone(), |v| format!("{arg} {v}"))),
                }
            }
            _ if name.is_empty() || BLOCKED.contains(&name) => cli.ignored.push(arg),
            _ => {
                // "--network-caching 3000": a number after an option is its value.
                match inline {
                    None if !negated && args.peek().is_some_and(|next| is_number(next)) => {
                        let value = args.next().unwrap_or_default();
                        cli.vlc_args.push(format!("{arg}={value}"));
                    }
                    _ => cli.vlc_args.push(arg),
                }
            }
        }
    }
    cli
}

impl Cli {
    /// Makes relative file paths absolute, from the folder ReVLC was
    /// started in: libVLC only opens absolute ones. URLs stay as they are.
    pub fn resolve_paths(&mut self, cwd: &std::path::Path) {
        for file in &mut self.files {
            if !file.contains("://") && std::path::Path::new(file).is_relative() {
                *file = cwd.join(&*file).to_string_lossy().into_owned();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Cli {
        parse(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn files_and_urls() {
        let cli = p(&["/Movies/a.mkv", "https://example.com/live.m3u8", "-", "relative.mp4"]);
        assert_eq!(cli.files, ["/Movies/a.mkv", "https://example.com/live.m3u8", "relative.mp4"]);
        assert_eq!(cli, Cli { files: cli.files.clone(), ..Default::default() });
    }

    #[test]
    fn handled_options() {
        let cli = p(&["--fullscreen", "--play-and-exit", "--start-time=90", "a.mkv"]);
        assert!(cli.fullscreen && cli.play_and_exit);
        assert_eq!(cli.media_options, [":start-time=90"]);
        assert_eq!(cli.files, ["a.mkv"]);
        assert!(p(&["-f"]).fullscreen);
        assert!(!p(&["--fullscreen", "--no-fullscreen"]).fullscreen);
    }

    #[test]
    fn times_take_separate_values() {
        let cli = p(&["--start-time", "12.5", "--stop-time", "60", "a.mkv"]);
        assert_eq!(cli.media_options, [":start-time=12.5", ":stop-time=60"]);
        assert_eq!(cli.files, ["a.mkv"]);
        // A file after the option isn't its value.
        let cli = p(&["--start-time", "a.mkv"]);
        assert_eq!(cli.files, ["a.mkv"]);
        assert_eq!(cli.ignored, ["--start-time"]);
        assert_eq!(p(&["--run-time=-5"]).ignored, ["--run-time=-5"]);
        assert_eq!(p(&["--start-time=soon"]).media_options, Vec::<String>::new());
    }

    #[test]
    fn other_options_go_to_libvlc() {
        let cli = p(&["--no-audio", "--network-caching", "3000", "--deinterlace-mode=yadif", "--no-video-title-show", "a.mkv"]);
        assert_eq!(cli.vlc_args, ["--no-audio", "--network-caching=3000", "--deinterlace-mode=yadif", "--no-video-title-show"]);
        assert_eq!(cli.files, ["a.mkv"]);
        // A flag followed by a file keeps the file.
        let cli = p(&["--loop", "b.mkv"]);
        assert_eq!((cli.vlc_args, cli.files), (vec!["--loop".to_string()], vec!["b.mkv".to_string()]));
    }

    #[test]
    fn unsafe_options_are_ignored() {
        let cli = p(&["--intf=macosx", "--vout=caopengllayer", "--sout=#std{access=file,dst=x.ts}", "--reset-config", "--help", "--no-embedded-video", "-I", "-vvv", "-psn_0_12345"]);
        assert!(cli.vlc_args.is_empty());
        assert_eq!(cli.ignored, ["--intf=macosx", "--vout=caopengllayer", "--sout=#std{access=file,dst=x.ts}", "--reset-config", "--help", "--no-embedded-video", "-I", "-vvv"]);
        assert_eq!(p(&["---", "--="]).ignored, ["---", "--="]);
    }

    #[test]
    fn relative_paths_become_absolute() {
        // An absolute path needs a drive letter on Windows.
        let absolute = if cfg!(windows) { r"C:\Movies\b.mkv" } else { "/Movies/b.mkv" };
        let mut cli = p(&["media/a.mkv", absolute, "https://example.com/c.m3u8"]);
        let cwd = std::env::temp_dir();
        cli.resolve_paths(&cwd);
        assert_eq!(cli.files[0], cwd.join("media/a.mkv").to_string_lossy());
        assert_eq!(&cli.files[1..], [absolute, "https://example.com/c.m3u8"]);
    }

    #[test]
    fn double_dash_ends_options() {
        let cli = p(&["--fullscreen", "--", "--weird name.mkv", "-f"]);
        assert!(cli.fullscreen);
        assert_eq!(cli.files, ["--weird name.mkv", "-f"]);
    }
}
