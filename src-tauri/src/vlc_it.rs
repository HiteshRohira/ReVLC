//! ReVLC's own code on top of a real libVLC: the wrapper, the effects model
//! that builds filter chains, tracks, chapters, thumbnails and snapshots,
//! checked by the pixels and samples libVLC actually produces. Not tests of
//! VLC itself: each one exercises something ReVLC decides.
//!
//! They need a libVLC 3 and FFmpeg (for the generated media), so they're
//! ignored by default. Run them one at a time, since libVLC keeps
//! process-wide state:
//!
//!   REVLC_VLC_DIR=src-tauri/vlc REVLC_TEST_FFMPEG=ffmpeg \
//!     cargo test vlc_it -- --ignored --test-threads=1
//!
//! (On Linux, leave REVLC_VLC_DIR unset to use the system libVLC.) Video goes
//! to memory through the same callbacks the Linux build draws with, audio to
//! a WAV file; nothing needs a screen, a GPU or a sound card.

use super::*;
use crate::effects::{Effects, Filter, StereoMode, Transform};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Frames = Arc<Mutex<Option<PresentedFrame>>>;

fn work() -> PathBuf {
    let dir = std::env::temp_dir().join("revlc-vlc-it");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Makes `name` with FFmpeg once per machine (the media is deterministic).
fn media(name: &str, args: &[&str]) -> PathBuf {
    let out = work().join(name);
    if !out.exists() {
        let ffmpeg = std::env::var("REVLC_TEST_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let tmp = work().join(format!("tmp-{name}"));
        let status = std::process::Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(args)
            .arg(&tmp)
            .status()
            .expect("FFmpeg runs (set REVLC_TEST_FFMPEG)");
        assert!(status.success(), "FFmpeg couldn't make {name}");
        std::fs::rename(&tmp, &out).unwrap();
    }
    out
}

fn pattern() -> PathBuf {
    media("pattern.mp4", &["-f", "lavfi", "-i", "testsrc2=size=320x180:rate=24", "-t", "30", "-c:v", "mpeg4", "-q:v", "3"])
}

/// Red on the left half, blue on the right: rotations and flips move them.
fn halves() -> PathBuf {
    media("halves.mp4", &[
        "-f", "lavfi", "-i", "color=c=red:size=160x180:rate=24", "-f", "lavfi", "-i", "color=c=blue:size=160x180:rate=24",
        "-filter_complex", "[0][1]hstack", "-t", "20", "-c:v", "mpeg4", "-q:v", "2",
    ])
}

/// A 440 Hz tone on the left channel only, silence on the right.
fn left_tone() -> PathBuf {
    media("left-tone.wav", &["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-af", "pan=stereo|c0=c0", "-t", "20"])
}

fn load(extra: &[&str]) -> Vlc {
    let args: Vec<String> = ["--no-video-title-show", "--avcodec-hw=none"].iter().chain(extra).map(|s| s.to_string()).collect();
    Vlc::load(None, &args, &[], &[]).unwrap_or_else(|e| panic!("libVLC didn't load ({}): {}", e.stage, e.message))
}

/// A player whose video goes to memory, like the Linux build's.
fn video_player() -> (Vlc, Frames) {
    let vlc = load(&["--no-audio"]);
    let frames: Frames = Arc::default();
    vlc.set_gtk_surface(frames.clone());
    (vlc, frames)
}

fn wait<T>(what: &str, secs: u64, mut f: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < end, "timed out after {secs} s waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn play(vlc: &Vlc, path: &Path) {
    vlc.open(path.to_str().unwrap(), &[], true).unwrap();
    wait("playback to start", 15, || (vlc.state() == State::Playing && vlc.time() > 0).then_some(()));
}

/// A frame shown after `after`: filter changes take a frame or two.
fn frame(frames: &Frames, after: Duration) -> (Vec<[u8; 3]>, i32, i32) {
    std::thread::sleep(after);
    let f = wait("a frame", 10, || frames.lock().unwrap().take());
    let mut px = Vec::with_capacity((f.width * f.height) as usize);
    for y in 0..f.height {
        let row = &f.pixels[(y * f.stride) as usize..];
        for x in 0..f.width as usize {
            // RV32: blue, green, red, unused.
            px.push([row[x * 4 + 2], row[x * 4 + 1], row[x * 4]]);
        }
    }
    (px, f.width, f.height)
}

fn mean(px: &[[u8; 3]]) -> [f64; 3] {
    let n = px.len() as f64;
    let mut s = [0.0; 3];
    for p in px {
        for c in 0..3 {
            s[c] += p[c] as f64;
        }
    }
    s.map(|v| v / n)
}

fn region(px: &[[u8; 3]], w: i32, h: i32, x0: f64, x1: f64) -> Vec<[u8; 3]> {
    let (a, b) = ((w as f64 * x0) as i32, (w as f64 * x1) as i32);
    (h / 4..3 * h / 4).flat_map(|y| (a..b).map(move |x| px[(y * w + x) as usize])).collect()
}

fn filter_on() -> Filter {
    Filter { enabled: true, values: Default::default() }
}

fn apply(vlc: &Vlc, fx: &Effects) {
    vlc.set_video_filters(&fx.video_chain());
}

#[test]
#[ignore = "needs libVLC and FFmpeg; see the module docs"]
fn vlc_it_loads_libvlc_3() {
    let vlc = load(&[]);
    let version = vlc.version.clone();
    assert!(version.starts_with("3."), "libVLC {version}");
}

#[test]
#[ignore = "needs libVLC and FFmpeg; see the module docs"]
fn vlc_it_rejects_a_broken_runtime() {
    let dir = work().join("broken-vlc");
    let lib = lib_dir(&dir);
    std::fs::create_dir_all(&lib).unwrap();
    let (core, main) = lib_names();
    std::fs::write(lib.join(core), b"not a library").unwrap();
    std::fs::write(lib.join(main), b"not a library").unwrap();
    // REVLC_VLC_DIR is the first place ReVLC looks.
    let good = std::env::var_os("REVLC_VLC_DIR");
    // SAFETY: tests run one at a time (see the module docs).
    unsafe { std::env::set_var("REVLC_VLC_DIR", &dir) };
    let result = Vlc::load(None, &[], &[], &[]);
    unsafe {
        match &good {
            Some(d) => std::env::set_var("REVLC_VLC_DIR", d),
            None => std::env::remove_var("REVLC_VLC_DIR"),
        }
    }
    let err = result.err().expect("a broken libVLC must not load");
    assert_eq!(err.stage, "load", "{}", err.message);
}

#[test]
#[ignore = "needs libVLC and FFmpeg; see the module docs"]
fn vlc_it_plays_seeks_and_reports() {
    let (vlc, frames) = video_player();
    play(&vlc, &pattern());
    assert!((vlc.length() - 30_000).abs() < 500, "length {}", vlc.length());
    assert!(vlc.seekable());
    wait("video", 5, || vlc.has_video().then_some(()));
    let (px, w, h) = frame(&frames, Duration::from_millis(200));
    // The decoder can pad the height to a multiple of 16; the memory output
    // passes the padded size through.
    assert!(w == 320 && (180..=192).contains(&h), "frame {w}×{h}");
    let colourful = px.iter().filter(|p| p.iter().max().unwrap() - p.iter().min().unwrap() > 100).count();
    assert!(colourful * 3 > px.len(), "the frame isn't the test pattern");
    vlc.set_time(20_000);
    wait("the seek", 5, || (vlc.time() >= 19_000).then_some(()));
    vlc.set_pause(true);
    wait("pause", 5, || (vlc.state() == State::Paused).then_some(()));
    let t = vlc.time();
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(vlc.time(), t, "the clock moved while paused");
}

#[test]
#[ignore = "needs libVLC and FFmpeg; see the module docs"]
fn vlc_it_invert_and_sepia_reach_the_picture() {
    let (vlc, frames) = video_player();
    play(&vlc, &halves());
    let (px, w, h) = frame(&frames, Duration::from_millis(300));
    let red = mean(&region(&px, w, h, 0.1, 0.4));
    assert!(red[0] > 180.0 && red[2] < 80.0, "left half {red:?}");

    let mut fx = Effects::default();
    assert!(fx.set_filter("invert", filter_on()));
    apply(&vlc, &fx);
    let (px, w, h) = frame(&frames, Duration::from_millis(500));
    let inv = mean(&region(&px, w, h, 0.1, 0.4));
    // Red inverted is cyan.
    assert!(inv[0] < 80.0 && inv[1] > 170.0 && inv[2] > 170.0, "inverted left half {inv:?}");

    let mut fx = Effects::default();
    assert!(fx.set_filter("sepia", filter_on()));
    apply(&vlc, &fx);
    let (px, w, h) = frame(&frames, Duration::from_millis(500));
    let blue = mean(&region(&px, w, h, 0.6, 0.9));
    // Sepia tints everything brown: no blue to speak of.
    assert!(blue[2] < blue[0], "sepia right half {blue:?}");
}

#[test]
#[ignore = "needs libVLC and FFmpeg; see the module docs"]
fn vlc_it_transform_rotates_and_flips() {
    let (vlc, frames) = video_player();
    play(&vlc, &halves());
    let mut fx = Effects { transform: Transform::FlipHorizontal, ..Default::default() };
    apply(&vlc, &fx);
    let (px, w, h) = frame(&frames, Duration::from_millis(600));
    let left = mean(&region(&px, w, h, 0.1, 0.4));
    assert!(left[2] > 180.0 && left[0] < 80.0, "flipped: the left half should be blue, is {left:?}");

    fx.transform = Transform::Rotate90;
    apply(&vlc, &fx);
    // The memory output keeps its size, so the turned picture is squeezed
    // into it: red, on the left, ends up at the top.
    let (px, w, h) = frame(&frames, Duration::from_millis(800));
    let px = &px;
    let rows = |y0: f64, y1: f64| -> Vec<[u8; 3]> {
        let (a, b) = ((h as f64 * y0) as i32, (h as f64 * y1) as i32);
        (a..b).flat_map(|y| (w / 4..3 * w / 4).map(move |x| px[(y * w + x) as usize])).collect()
    };
    let (top, bottom) = (mean(&rows(0.05, 0.35)), mean(&rows(0.65, 0.9)));
    assert!(top[0] > 150.0 && top[2] < 100.0 && bottom[2] > 150.0, "rotated 90°: top {top:?}, bottom {bottom:?}");
}

#[test]
#[ignore = "needs libVLC and FFmpeg; see the module docs"]
fn vlc_it_adjust_saturation_zero_is_grey() {
    let (vlc, frames) = video_player();
    play(&vlc, &pattern());
    let mut fx = Effects::default();
    fx.adjust.enabled = true;
    fx.adjust.saturation = 0.0;
    vlc.set_adjust(&fx.adjust);
    apply(&vlc, &fx);
    let (px, _, _) = frame(&frames, Duration::from_millis(600));
    let grey = px.iter().filter(|p| p.iter().max().unwrap() - p.iter().min().unwrap() < 24).count();
    assert!(grey * 10 > px.len() * 9, "only {}% grey", grey * 100 / px.len());
}

/// Plays `path` into a WAV with ReVLC's audio settings, then reads it back.
fn record(path: &Path, setup: impl FnOnce(&Vlc)) -> (Vec<f32>, Vec<f32>) {
    let wav = work().join(format!("out-{}.wav", std::process::id()));
    let _ = std::fs::remove_file(&wav);
    let file = format!("--audiofile-file={}", wav.display());
    let vlc = load(&["--no-video", "--aout=afile", &file, "--audiofile-format=s16", "--audiofile-channels=2", "--audiofile-wav"]);
    vlc.open(path.to_str().unwrap(), &[], true).unwrap();
    wait("audio to play", 15, || (vlc.time() > 200).then_some(()));
    // Like ReVLC, which applies audio settings once the output exists.
    setup(&vlc);
    std::thread::sleep(Duration::from_millis(500));
    let mark = vlc.time();
    wait("three seconds of audio", 15, || (vlc.time() > mark + 3000).then_some(()));
    vlc.stop();
    drop(vlc);
    let bytes = std::fs::read(&wav).expect("libVLC wrote the WAV");
    let data = bytes.windows(4).position(|w| w == b"data").expect("a WAV data chunk") + 8;
    let samples: Vec<i16> = bytes[data..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    // From a moment after the settings were applied.
    let skip = ((mark + 300) * 48) as usize;
    let (l, r) = samples.chunks_exact(2).skip(skip).map(|s| (s[0] as f32 / 32768.0, s[1] as f32 / 32768.0)).unzip();
    (l, r)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// The dominant frequency, from upward zero crossings (the tones are pure).
fn frequency(x: &[f32], rate: f32) -> f32 {
    let crossings = x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    crossings as f32 * rate / x.len() as f32
}

#[test]
#[ignore = "needs libVLC and FFmpeg; see the module docs"]
fn vlc_it_stereo_modes_route_channels() {
    let (l, r) = record(&left_tone(), |_| {});
    assert!(rms(&l) > 0.05 && rms(&r) < 0.01, "plain stereo: left {} right {}", rms(&l), rms(&r));
    let (l, r) = record(&left_tone(), |vlc| {
        vlc.set_stereo_mode(StereoMode::Mono.code());
    });
    let diff = l.iter().zip(&r).map(|(a, b)| (a - b).abs()).fold(0.0_f32, f32::max);
    assert!(rms(&r) > 0.02 && diff < 0.02, "mono: left {} right {} max diff {diff}", rms(&l), rms(&r));
}

#[test]
#[ignore = "needs libVLC and FFmpeg; see the module docs"]
fn vlc_it_pitch_filter_shifts_an_octave() {
    let (l, _) = record(&left_tone(), |_| {});
    let base = frequency(&l, 48_000.0);
    assert!((base - 440.0).abs() < 15.0, "base {base} Hz");
    let mut fx = Effects::default();
    let mut pitch = filter_on();
    pitch.values.insert("pitch-shift".into(), 12.0);
    assert!(fx.set_filter("pitch", pitch));
    let (l, _) = record(&left_tone(), |vlc| vlc.set_audio_filters(&fx.audio_chain(), &fx.audio_params(), true));
    let shifted = frequency(&l, 48_000.0);
    assert!((shifted - 880.0).abs() < 30.0, "+12 semitones gave {shifted} Hz");
}

#[test]
#[ignore = "needs libVLC and FFmpeg; see the module docs"]
fn vlc_it_equalizer_presets_change_the_level() {
    let (flat, _) = record(&left_tone(), |_| {});
    let vlc = load(&["--no-audio"]);
    let presets = vlc.equalizer_presets();
    assert!(presets.len() >= 18, "{} presets", presets.len());
    drop(vlc);
    // A preset that cuts the mid band, where 440 Hz is.
    let (cut, _) = record(&left_tone(), |vlc| {
        let bands: Vec<f32> = vlc.equalizer_bands().iter().map(|&hz| if (250.0..=1000.0).contains(&hz) { -15.0 } else { 0.0 }).collect();
        vlc.set_equalizer(Some((0.0, &bands)));
    });
    assert!(rms(&cut) < rms(&flat) * 0.5, "cutting 440 Hz by 15 dB: {} vs {}", rms(&cut), rms(&flat));
}

#[test]
#[ignore = "needs libVLC and FFmpeg; see the module docs"]
fn vlc_it_tracks_chapters_and_subtitles() {
    let srt = work().join("line.srt");
    std::fs::write(&srt, "1\n00:00:00,000 --> 00:01:00,000\nSUBTITLE\n").unwrap();
    let meta = work().join("chapters.txt");
    std::fs::write(&meta, ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=10000\ntitle=One\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=10000\nEND=20000\ntitle=Two\n").unwrap();
    let file = media("tracks.mkv", &[
        "-f", "lavfi", "-i", "color=c=0x204060:size=320x180:rate=24", "-f", "lavfi", "-i", "sine=frequency=440",
        "-f", "lavfi", "-i", "sine=frequency=880", "-i", meta.to_str().unwrap(),
        "-map", "0", "-map", "1", "-map", "2", "-map_metadata", "3", "-t", "20", "-c:v", "mpeg4", "-c:a", "aac",
        "-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "language=fre",
    ]);
    let vlc = load(&["--aout=dummy"]);
    let frames: Frames = Arc::default();
    vlc.set_gtk_surface(frames.clone());
    play(&vlc, &file);
    wait("tracks", 5, || (vlc.audio_tracks().len() >= 2).then_some(()));
    let audio: Vec<i32> = vlc.audio_tracks().iter().map(|t| t.id).filter(|&id| id >= 0).collect();
    assert_eq!(audio.len(), 2, "{:?}", vlc.audio_tracks());
    vlc.set_audio_track(audio[1]);
    wait("the second audio track", 5, || (vlc.audio_track() == audio[1]).then_some(()));
    let chapters = vlc.chapters();
    // libVLC keeps a leading space from the container's chapter title.
    assert_eq!(chapters.iter().map(|c| c.name.trim()).collect::<Vec<_>>(), ["One", "Two"]);
    vlc.set_chapter(1);
    wait("chapter two", 5, || (vlc.time() >= 9_500).then_some(()));
    // An external subtitle, selected and drawn into the frame.
    // As ReVLC does: libVLC takes subtitles by URI, not path.
    vlc.add_subtitle(&crate::playlist_file::file_uri(srt.to_str().unwrap()), true).unwrap();
    wait("the subtitle track", 5, || (vlc.subtitle_track() >= 0).then_some(()));
    let white = wait("subtitle text in the frame", 10, || {
        let (px, w, h) = frame(&frames, Duration::from_millis(200));
        let n = (h * 2 / 3..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|&(x, y)| px[(y * w + x) as usize].iter().all(|&c| c > 180)).count();
        // The background (dark blue) has none; small text has a few dozen.
        (n > 15).then_some(n)
    });
    assert!(white > 15);
}

#[test]
#[ignore = "needs libVLC and FFmpeg; see the module docs"]
fn vlc_it_thumbnails_follow_the_file() {
    let vlc = load(&["--no-audio"]);
    let path = work().join("replaced.mp4");
    std::fs::copy(halves(), &path).unwrap();
    let out = work().join("thumb.jpg");
    vlc.thumbnail(path.to_str().unwrap(), &out, 0.5, 160).unwrap();
    let thumb = image::open(&out).unwrap().to_rgb8();
    let left = thumb.get_pixel(20, thumb.height() / 2);
    assert!(left[0] > 150 && left[2] < 100, "thumbnail left {left:?}");
    // The same path, other content: the next thumbnail shows the new file.
    let other = media("green.mp4", &["-f", "lavfi", "-i", "color=c=green:size=320x180:rate=24", "-t", "20", "-c:v", "mpeg4"]);
    std::fs::copy(other, &path).unwrap();
    vlc.thumbnail(path.to_str().unwrap(), &out, 0.5, 160).unwrap();
    let thumb = image::open(&out).unwrap().to_rgb8();
    let px = thumb.get_pixel(20, thumb.height() / 2);
    assert!(px[1] > 100 && px[0] < 100, "after replacing the file the thumbnail is {px:?}");
}

#[test]
#[ignore = "needs libVLC and FFmpeg; see the module docs"]
fn vlc_it_opens_long_unicode_paths_and_local_hls() {
    // The app is long-path aware on Windows through its manifest
    // (windows/app.manifest), which this test executable doesn't have, so
    // Windows gets a Unicode path under 260 characters here; the Cua suite
    // covers long paths with the real app.
    let segments = if cfg!(windows) { 3 } else { 12 };
    let deep = work().join(vec!["Ünïcødé 視頻 テスト — segment"; segments].join(std::path::MAIN_SEPARATOR_STR));
    std::fs::create_dir_all(&deep).unwrap();
    let file = deep.join("長いファイル名.mp4");
    std::fs::copy(pattern(), &file).unwrap();
    let chars = file.to_string_lossy().chars().count();
    assert!(if cfg!(windows) { chars < 260 } else { chars > 300 }, "{chars} characters");
    let (vlc, _frames) = video_player();
    play(&vlc, &file);

    let hls = work().join("hls");
    std::fs::create_dir_all(&hls).unwrap();
    let list = hls.join("stream.m3u8");
    if !list.exists() {
        let ffmpeg = std::env::var("REVLC_TEST_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let ok = std::process::Command::new(ffmpeg)
            .current_dir(&hls)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=24", "-t", "12"])
            .args(["-c:v", "mpeg4", "-f", "hls", "-hls_time", "2", "-hls_list_size", "0", "stream.m3u8"])
            .status()
            .unwrap()
            .success();
        assert!(ok);
    }
    play(&vlc, &list);
    assert!(vlc.length() > 10_000, "HLS length {}", vlc.length());
}
