//! Reproducible benchmarks for ReVLC architecture stages.
//! Run with:
//!   REVLC_VLC_DIR=src-tauri/vlc cargo test --lib bench:: -- --ignored --nocapture --test-threads=1

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::engine::State;
use crate::keyframes;
use crate::scrub;
use crate::vlc::Vlc;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn bundled_ffmpeg() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("ffmpeg-"))
                && p.metadata().is_ok_and(|m| m.len() > 0)
            {
                return p;
            }
        }
    }
    PathBuf::from("ffmpeg")
}

fn collect_bench_media() -> Vec<PathBuf> {
    let root = repo_root().join("testmedia");
    let mut files = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().and_then(|s| s.to_str()).is_some_and(|ext| {
                    matches!(ext.to_ascii_lowercase().as_str(), "mp4" | "mkv" | "webm" | "mp3" | "flac" | "m4a" | "ogg")
                }) {
                    out.push(p);
                }
            }
        }
    }
    walk(&root, &mut files);
    files.sort();

    // Generate 6 deterministic tagged audio files (MP3, FLAC, M4A, OGG) once in temp dir.
    let audio_dir = std::env::temp_dir().join("revlc-bench-audio");
    let _ = std::fs::create_dir_all(&audio_dir);
    let specs: &[(&str, &[&str])] = &[
        ("track01.mp3", &["-c:a", "libmp3lame", "-b:a", "128k"]),
        ("track02.mp3", &["-c:a", "libmp3lame", "-b:a", "128k"]),
        ("track03.flac", &["-c:a", "flac"]),
        ("track04.flac", &["-c:a", "flac"]),
        ("track05.m4a", &["-c:a", "aac", "-b:a", "128k"]),
        ("track06.ogg", &["-c:a", "libvorbis"]),
    ];
    let ffmpeg = PathBuf::from("ffmpeg");
    for (i, (name, codec)) in specs.iter().enumerate() {
        let out = audio_dir.join(name);
        if !out.exists() {
            let mut cmd = std::process::Command::new(&ffmpeg);
            cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-t", "5", "-i", "sine=frequency=440:sample_rate=44100"]);
            cmd.args(*codec);
            cmd.args([
                "-metadata", &format!("title=Bench Track {}", i + 1),
                "-metadata", "artist=Bench Artist",
                "-metadata", "album=Bench Album",
                "-metadata", &format!("track={}/6", i + 1),
                "-metadata", "date=2025",
            ]);
            cmd.arg(&out);
            let _ = cmd.status();
        }
        if out.exists() {
            files.push(out);
        }
    }
    files
}

fn percentile_ms(samples: &mut [u128], pct: f64) -> u128 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let idx = (((samples.len() - 1) as f64) * pct).round() as usize;
    samples[idx.min(samples.len() - 1)]
}

#[test]
#[ignore = "benchmark"]
fn bench_baseline_all() {
    let vlc_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vlc");
    // SAFETY: single-threaded benchmark test.
    unsafe { std::env::set_var("REVLC_VLC_DIR", &vlc_dir) };

    // 1. Metadata scan throughput (sequential Vlc::parse)
    let load_start = Instant::now();
    let vlc = Vlc::load(None, &["--no-audio".into(), "--no-video-title-show".into()], &[], &[])
        .unwrap_or_else(|e| panic!("Vlc::load failed: {}", e.message));
    let load_ms = load_start.elapsed().as_secs_f64() * 1000.0;
    println!("[BENCH] Vlc::load (in-process): {load_ms:.2} ms");

    let media_files = collect_bench_media();
    let meta_start = Instant::now();
    let mut parsed_ok = 0usize;
    for path in &media_files {
        if vlc.parse(path.to_str().unwrap(), 5000).is_ok() {
            parsed_ok += 1;
        }
    }
    let meta_elapsed = meta_start.elapsed();
    let meta_ms = meta_elapsed.as_secs_f64() * 1000.0;
    let files_per_sec = media_files.len() as f64 / meta_elapsed.as_secs_f64();
    println!(
        "[BENCH] Baseline Vlc::parse metadata scan: {}/{} files in {:.1} ms ({:.2} files/sec, {:.1} ms/file)",
        parsed_ok,
        media_files.len(),
        meta_ms,
        files_per_sec,
        meta_ms / media_files.len().max(1) as f64
    );

    // 2. Seek-bar preview TTFP & hover p95 latency (current 4-part FFmpeg scrub_pass)
    let ffmpeg = bundled_ffmpeg();
    let targets = [
        ("1080p H.264 MP4", repo_root().join("testmedia/test-1080p.mp4"), 60_000i64),
        ("1080p H.264 MKV", repo_root().join("testmedia/Tears of Steel.mkv"), 150_046i64),
        ("4K HEVC MP4", repo_root().join("testmedia/promo/Webb - Cassiopeia A.mp4"), 30_000i64),
        ("4K H.264 MP4", repo_root().join("testmedia/promo/NASA - Solar flare (SDO, 4K).mp4"), 79_566i64),
    ];
    let tmp_dir = std::env::temp_dir().join("revlc-bench-scrub-baseline");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let _ = std::fs::create_dir_all(&tmp_dir);

    for (label, path, length) in &targets {
        if !path.exists() {
            continue;
        }
        let step = scrub::step(*length);
        let count = (*length - 1) / step + 1;
        let start = Instant::now();
        let ttfp: Arc<Mutex<Option<Duration>>> = Arc::new(Mutex::new(None));
        let slot_times: Arc<Mutex<std::collections::HashMap<i64, Duration>>> = Arc::new(Mutex::new(Default::default()));
        let made: Arc<Mutex<HashSet<i64>>> = Arc::new(Mutex::new(HashSet::new()));
        let accel = keyframes::accel(&ffmpeg, path.to_str().unwrap(), 0, scrub::WIDTH);
        let parts = if accel.gpu() { 4 } else { 2 };
        let span = *length / parts + 1;

        let mut handles = Vec::new();
        for part in 0..parts {
            let ffmpeg = ffmpeg.clone();
            let uri = path.to_string_lossy().into_owned();
            let accel = accel.clone();
            let ttfp = ttfp.clone();
            let slot_times = slot_times.clone();
            let made = made.clone();
            let tmp_dir = tmp_dir.clone();
            handles.push(thread::spawn(move || {
                let _ = keyframes::pass(&ffmpeg, &uri, 0, &accel, part * span, span, scrub::WIDTH, |at, jpeg| {
                    let k = ((at + step / 2) / step).clamp(0, count - 1);
                    if !made.lock().unwrap().insert(k) {
                        return true;
                    }
                    let out = tmp_dir.join(format!("slot-{part}-{k}.jpg"));
                    let _ = std::fs::write(&out, &jpeg);
                    let elapsed = start.elapsed();
                    let mut first = ttfp.lock().unwrap();
                    if first.is_none() {
                        *first = Some(elapsed);
                    }
                    slot_times.lock().unwrap().insert(k, elapsed);
                    true
                });
            }));
        }
        for h in handles {
            let _ = h.join();
        }
        let total = start.elapsed();
        let first_ms = ttfp.lock().unwrap().map_or(0, |d| d.as_millis());
        let times_map = slot_times.lock().unwrap();
        // Simulate hovering across 19 evenly spaced points (5%..95%) from run start
        let mut hover_latencies = Vec::new();
        for i in 1..20 {
            let target_ms = (*length * i) / 20;
            let k = (target_ms / step).clamp(0, count - 1);
            // Nearest slot within reach, matching UI nearestScrubFrame
            let lat = [k, k - 1, k + 1]
                .into_iter()
                .filter_map(|s| times_map.get(&s).copied())
                .min()
                .unwrap_or(total);
            hover_latencies.push(lat.as_millis());
        }
        let p50 = percentile_ms(&mut hover_latencies, 0.50);
        let p95 = percentile_ms(&mut hover_latencies, 0.95);
        println!(
            "[BENCH] Baseline Scrub ({label}): TTFP={first_ms} ms, hover_p50={p50} ms, hover_p95={p95} ms, full_pass={} ms, slots={}/{}",
            total.as_millis(),
            times_map.len(),
            count
        );
    }
    let _ = std::fs::remove_dir_all(&tmp_dir);

    // 3. Playback frame drop rate under preview load
    let play_file = repo_root().join("testmedia/test-1080p.mp4");
    let scrub_file = repo_root().join("testmedia/promo/NASA - Solar flare (SDO, 4K).mp4");
    if play_file.exists() && scrub_file.exists() {
        let frames = Arc::new(Mutex::new(None));
        vlc.set_gtk_surface(frames);
        vlc.open(play_file.to_str().unwrap(), &[], true).unwrap();
        let wait_start = Instant::now();
        while vlc.state() != State::Playing && wait_start.elapsed() < Duration::from_secs(10) {
            thread::sleep(Duration::from_millis(20));
        }
        thread::sleep(Duration::from_millis(500));
        let before = vlc.stats().unwrap_or_default();

        // Launch 4-part FFmpeg preview load while playing for ~4 seconds
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut workers = Vec::new();
        let accel = keyframes::accel(&ffmpeg, scrub_file.to_str().unwrap(), 0, scrub::WIDTH);
        for part in 0..4i64 {
            let ffmpeg = ffmpeg.clone();
            let uri = scrub_file.to_string_lossy().into_owned();
            let accel = accel.clone();
            let stop = stop.clone();
            workers.push(thread::spawn(move || {
                let _ = keyframes::pass(&ffmpeg, &uri, 0, &accel, part * 20_000, 20_000, scrub::WIDTH, |_, _| {
                    !stop.load(std::sync::atomic::Ordering::Relaxed)
                });
            }));
        }
        thread::sleep(Duration::from_secs(4));
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let after = vlc.stats().unwrap_or_default();
        vlc.stop();
        for w in workers {
            let _ = w.join();
        }
        let displayed = (after.displayed_pictures - before.displayed_pictures).max(0);
        let lost = (after.lost_pictures - before.lost_pictures).max(0);
        let total = displayed + lost;
        let drop_pct = if total > 0 { (lost as f64 / total as f64) * 100.0 } else { 0.0 };
        println!(
            "[BENCH] Baseline Playback under preview load (4s): displayed={displayed}, lost={lost}, drop_rate={drop_pct:.2}%"
        );
    }
}

#[test]
#[ignore = "benchmark"]
fn bench_after_all() {
    use crate::container;
    use crate::preview;

    let vlc_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vlc");
    // SAFETY: single-threaded benchmark test.
    unsafe { std::env::set_var("REVLC_VLC_DIR", &vlc_dir) };

    // 1. Cold/in-process Vlc::load after plugin pruning
    let load_start = Instant::now();
    let vlc = Vlc::load(None, &["--no-audio".into(), "--no-video-title-show".into()], &[], &[])
        .unwrap_or_else(|e| panic!("Vlc::load failed: {}", e.message));
    let load_ms = load_start.elapsed().as_secs_f64() * 1000.0;
    println!("[BENCH] After Vlc::load (in-process, pruned plugins): {load_ms:.2} ms");

    // 2. Parallel pure-Rust metadata scan (4 workers, matching MetaScanner pool)
    let media_files = collect_bench_media();
    let art_dir = std::env::temp_dir().join("revlc-bench-after-art");
    let _ = std::fs::remove_dir_all(&art_dir);
    let _ = std::fs::create_dir_all(&art_dir);
    let vlc = Arc::new(vlc);

    let meta_start = Instant::now();
    let queue = Arc::new(Mutex::new(std::collections::VecDeque::from(media_files.clone())));
    let parsed_ok = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let fallback_used = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut workers = Vec::new();
    for _ in 0..4 {
        let queue = Arc::clone(&queue);
        let parsed_ok = Arc::clone(&parsed_ok);
        let fallback_used = Arc::clone(&fallback_used);
        let art_dir = art_dir.clone();
        let vlc = Arc::clone(&vlc);
        workers.push(thread::spawn(move || {
            loop {
                let next = { queue.lock().unwrap().pop_front() };
                let Some(path) = next else { break };
                if container::parse_metadata(&path, &art_dir).is_some() {
                    parsed_ok.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                } else if vlc.parse(path.to_str().unwrap(), 5000).is_ok() {
                    fallback_used.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    parsed_ok.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }));
    }
    for w in workers {
        let _ = w.join();
    }
    let meta_elapsed = meta_start.elapsed();
    let meta_ms = meta_elapsed.as_secs_f64() * 1000.0;
    let files_per_sec = media_files.len() as f64 / meta_elapsed.as_secs_f64();
    println!(
        "[BENCH] After Pure-Rust parallel metadata scan: {}/{} files in {:.1} ms ({:.2} files/sec, {:.2} ms/file, vlc_fallbacks={})",
        parsed_ok.load(std::sync::atomic::Ordering::Relaxed),
        media_files.len(),
        meta_ms,
        files_per_sec,
        meta_ms / media_files.len().max(1) as f64,
        fallback_used.load(std::sync::atomic::Ordering::Relaxed),
    );
    let _ = std::fs::remove_dir_all(&art_dir);

    // 3. Seek-bar preview TTFP, hover p50/p95, and full pass via warm native VideoToolbox session + in-memory LRU cache
    let targets = [
        ("1080p H.264 MP4", repo_root().join("testmedia/test-1080p.mp4"), 60_000i64),
        ("1080p H.264 MKV", repo_root().join("testmedia/Tears of Steel.mkv"), 150_046i64),
        ("4K HEVC MP4", repo_root().join("testmedia/promo/Webb - Cassiopeia A.mp4"), 30_000i64),
        ("4K H.264 MP4", repo_root().join("testmedia/promo/NASA - Solar flare (SDO, 4K).mp4"), 79_566i64),
    ];
    for (label, path, length) in &targets {
        if !path.exists() {
            continue;
        }
        let step = scrub::step(*length);
        let count = (*length - 1) / step + 1;
        let cache = preview::cache::ScrubCache::default();
        let priority = preview::PriorityQueue::new(count);

        let start = Instant::now();
        let Ok(mut session) = preview::WarmSession::open(path, 0) else {
            println!("[BENCH] After WarmSession Scrub ({label}): native backend unavailable on this platform");
            continue;
        };
        let mut ttfp: Option<Duration> = None;
        let mut slot_times: std::collections::HashMap<i64, Duration> = Default::default();
        while let Some(k) = priority.pop_next() {
            let target_ms = scrub::time_at(*length, step, k);
            if let Ok((_, jpeg)) = session.decode_nearest(target_ms, scrub::WIDTH) {
                cache.insert(1, k, jpeg);
                let elapsed = start.elapsed();
                if ttfp.is_none() {
                    ttfp = Some(elapsed);
                }
                slot_times.insert(k, elapsed);
            }
        }
        let total = start.elapsed();
        let first_ms = ttfp.map_or(0, |d| d.as_millis());
        let mut hover_latencies = Vec::new();
        for i in 1..20 {
            let target_ms = (*length * i) / 20;
            let k = (target_ms / step).clamp(0, count - 1);
            let lat = [k, k - 1, k + 1]
                .into_iter()
                .filter_map(|s| slot_times.get(&s).copied())
                .min()
                .unwrap_or(total);
            hover_latencies.push(lat.as_millis());
        }
        let p50 = percentile_ms(&mut hover_latencies, 0.50);
        let p95 = percentile_ms(&mut hover_latencies, 0.95);

        // Also measure on-demand warm hover decode latency (random single hover frame on a fresh warm session)
        let mut on_demand_ms = Vec::new();
        if let Ok(mut hover_session) = preview::WarmSession::open(path, 0) {
            for i in 1..10 {
                let target_ms = (*length * i) / 10;
                let t0 = Instant::now();
                if hover_session.decode_nearest(target_ms, scrub::WIDTH).is_ok() {
                    on_demand_ms.push(t0.elapsed().as_millis());
                }
            }
        }
        let on_demand_p50 = percentile_ms(&mut on_demand_ms, 0.50);
        let on_demand_p95 = percentile_ms(&mut on_demand_ms, 0.95);

        println!(
            "[BENCH] After WarmSession Scrub ({label}): TTFP={first_ms} ms, sweep_hover_p50={p50} ms, sweep_hover_p95={p95} ms, full_pass={} ms, on_demand_p50={on_demand_p50} ms, on_demand_p95={on_demand_p95} ms, slots={}/{}",
            total.as_millis(),
            slot_times.len(),
            count
        );
    }

    // 4. Playback frame drop rate under warm-session 4K preview load
    let play_file = repo_root().join("testmedia/test-1080p.mp4");
    let scrub_file = repo_root().join("testmedia/promo/NASA - Solar flare (SDO, 4K).mp4");
    if play_file.exists() && scrub_file.exists() {
        let frames = Arc::new(Mutex::new(None));
        vlc.set_gtk_surface(frames);
        vlc.open(play_file.to_str().unwrap(), &[], true).unwrap();
        let wait_start = Instant::now();
        while vlc.state() != State::Playing && wait_start.elapsed() < Duration::from_secs(10) {
            thread::sleep(Duration::from_millis(20));
        }
        thread::sleep(Duration::from_millis(500));
        let before = vlc.stats().unwrap_or_default();

        // Repeatedly run WarmSession 4K preview passes while playing for 4 seconds
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_w = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut decoded = 0usize;
            while !stop_w.load(std::sync::atomic::Ordering::Relaxed) {
                if let Ok(mut s) = preview::WarmSession::open(&scrub_file, 0) {
                    for k in 0..40 {
                        if stop_w.load(std::sync::atomic::Ordering::Relaxed) {
                            break;
                        }
                        if s.decode_nearest(k * 2000, scrub::WIDTH).is_ok() {
                            decoded += 1;
                        }
                    }
                }
            }
            decoded
        });
        thread::sleep(Duration::from_secs(4));
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let after = vlc.stats().unwrap_or_default();
        vlc.stop();
        let decoded = worker.join().unwrap_or(0);
        let displayed = (after.displayed_pictures - before.displayed_pictures).max(0);
        let lost = (after.lost_pictures - before.lost_pictures).max(0);
        let total = displayed + lost;
        let drop_pct = if total > 0 { (lost as f64 / total as f64) * 100.0 } else { 0.0 };
        println!(
            "[BENCH] After Playback under WarmSession 4K preview load (4s, {decoded} preview frames): displayed={displayed}, lost={lost}, drop_rate={drop_pct:.2}%"
        );
    }
}
