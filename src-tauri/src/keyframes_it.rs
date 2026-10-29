//! The seek bar's preview frames, made by the FFmpeg ReVLC ships (the
//! system one on Linux): the decoder ReVLC picks on this machine, the frames
//! and times it hands over, and what happens when a decoder can't work.
//! Ignored by default, like vlc_it.rs: they need FFmpeg.
//!
//!   REVLC_TEST_FFMPEG=ffmpeg cargo test keyframes_it -- --ignored
//!
//! In CI the runners have no GPU video decoding, so the decoder ReVLC picks
//! there has to fall back to one that works: a machine without a usable GPU
//! is exactly where the Windows keyframe bug was found.

use super::*;
use crate::scrub::WIDTH;
use crate::test_media::{known_bug_on_some_machines, media};
use std::path::PathBuf;

fn ffmpeg() -> PathBuf {
    std::env::var_os("REVLC_TEST_FFMPEG").map(PathBuf::from).unwrap_or_else(|| "ffmpeg".into())
}

/// 10 s of H.264 at 25 fps with a keyframe every second, as most films
/// are, so a GPU decoder could take it.
fn ten_seconds() -> PathBuf {
    media("keyframes-10s.mp4", &[
        "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=25", "-t", "10",
        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-g", "25", "-keyint_min", "25", "-sc_threshold", "0",
    ])
}

/// Two video tracks: red, then a smaller blue one.
fn two_tracks() -> PathBuf {
    media("keyframes-two-tracks.mkv", &[
        "-f", "lavfi", "-i", "color=c=red:size=640x360:rate=25", "-f", "lavfi", "-i", "color=c=blue:size=320x240:rate=25",
        "-map", "0", "-map", "1", "-t", "4", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-g", "25",
    ])
}

fn passes(accel: &Accel, video: &std::path::Path, track: usize, len: i64) -> Result<Vec<(i64, image::RgbImage)>, String> {
    let mut got = Vec::new();
    pass(&ffmpeg(), video.to_str().unwrap(), track, accel, 0, len, WIDTH, |at, jpeg| {
        let img = image::load_from_memory(&jpeg).expect("a JPEG").to_rgb8();
        got.push((at, img));
        true
    })?;
    Ok(got)
}

fn check_frames(got: &[(i64, image::RgbImage)]) {
    let times: Vec<i64> = got.iter().map(|g| g.0).collect();
    assert_eq!(times.len(), 10, "one frame per keyframe, got times {times:?}");
    for (i, t) in times.iter().enumerate() {
        assert!((t - i as i64 * 1000).abs() <= 40, "keyframe {i} at {t} ms");
    }
    for (_, img) in got {
        assert_eq!(img.width(), WIDTH, "preview width");
        assert_eq!(img.height(), WIDTH * 360 / 640, "preview height keeps the shape");
    }
}

#[test]
#[ignore = "needs FFmpeg; see the module docs"]
fn keyframes_it_the_decoder_revlc_picks_makes_previews() {
    let video = ten_seconds();
    let accel = accel(&ffmpeg(), video.to_str().unwrap(), 0, WIDTH);
    eprintln!("decoder picked: {accel:?}");
    let run = move || {
        let got = passes(&accel, &video, 0, 10_000).unwrap_or_else(|e| panic!("keyframe pass failed with {accel:?}: {e}"));
        check_frames(&got);
    };
    if cfg!(windows) {
        // ReVLC picks D3D11VA on Windows without trying it; with no GPU
        // decoding (CI, a VM), every pass fails. Fixed in PR #10.
        known_bug_on_some_machines("Windows: the keyframe pass uses D3D11VA without checking it works", run);
    } else {
        run();
    }
}

#[test]
#[ignore = "needs FFmpeg; see the module docs"]
fn keyframes_it_the_cpu_makes_the_same_previews() {
    check_frames(&passes(&Accel::Cpu, &ten_seconds(), 0, 10_000).unwrap());
}

#[test]
#[ignore = "needs FFmpeg; see the module docs"]
fn keyframes_it_a_gpu_that_cant_decode_is_not_picked() {
    let video = ten_seconds();
    let video = video.to_str().unwrap();
    assert!(!decodes(&ffmpeg(), video, 0, &Accel::Vaapi("/dev/dri/no-such-gpu".into()), WIDTH), "a missing GPU can't decode");
    assert!(decodes(&ffmpeg(), video, 0, &Accel::Cpu, WIDTH), "the CPU can");
    let picked = accel(&ffmpeg(), video, 0, WIDTH);
    let works = move || assert!(decodes(&ffmpeg(), video, 0, &picked, WIDTH), "ReVLC picked {picked:?}, which can't decode here");
    if cfg!(windows) {
        known_bug_on_some_machines("Windows: the keyframe pass uses D3D11VA without checking it works", works);
    } else {
        works();
    }
}

#[test]
#[ignore = "needs FFmpeg; see the module docs"]
fn keyframes_it_a_pass_covers_its_stretch_and_stops_when_told() {
    let video = ten_seconds();
    let mut times = Vec::new();
    pass(&ffmpeg(), video.to_str().unwrap(), 0, &Accel::Cpu, 4_000, 3_000, WIDTH, |at, _| {
        times.push(at);
        true
    })
    .unwrap();
    // Times as in the file, not from the seek point.
    assert_eq!(times.len(), 3, "keyframes 4, 5 and 6 s: {times:?}");
    assert!((times[0] - 4_000).abs() <= 40, "{times:?}");

    let mut n = 0;
    pass(&ffmpeg(), video.to_str().unwrap(), 0, &Accel::Cpu, 0, 10_000, WIDTH, |_, _| {
        n += 1;
        n < 2
    })
    .expect("stopping early isn't an error");
    assert_eq!(n, 2);
}

#[test]
#[ignore = "needs FFmpeg; see the module docs"]
fn keyframes_it_the_second_video_track_has_its_own_previews() {
    let got = passes(&Accel::Cpu, &two_tracks(), 1, 4_000).unwrap();
    assert!(!got.is_empty(), "no frames from track 2");
    let img = &got[0].1;
    assert_eq!(img.height(), WIDTH * 240 / 320, "track 2's shape");
    let px = img.get_pixel(img.width() / 2, img.height() / 2).0;
    assert!(px[2] > 180 && px[0] < 80, "track 2 is blue, got {px:?}");
}

#[test]
#[ignore = "needs FFmpeg; see the module docs"]
fn keyframes_it_a_broken_file_is_an_error_not_a_hang() {
    let broken = crate::test_media::work().join("keyframes-broken.mp4");
    let mut bytes = std::fs::read(ten_seconds()).unwrap();
    bytes.truncate(bytes.len() / 3);
    bytes[..64].fill(0);
    std::fs::write(&broken, bytes).unwrap();
    let start = std::time::Instant::now();
    let result = passes(&Accel::Cpu, &broken, 0, 10_000);
    assert!(start.elapsed().as_secs() < 30, "took {:?}", start.elapsed());
    assert!(result.is_err() || result.as_ref().is_ok_and(|g| g.is_empty()), "a broken file made frames: {:?}", result.map(|g| g.len()));
}
