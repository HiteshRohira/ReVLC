//! Seek bar preview frames from FFmpeg (a sidecar next to ReVLC on macOS and
//! Windows, a package dependency on Linux): one process decodes just the keyframes of a
//! stretch of video, on the GPU where it can (VA-API, VideoToolbox, D3D11VA),
//! and hands them over as JPEGs with their times. A 2-hour 4K HEVC film
//! takes about 11 s on a laptop's integrated GPU, against a minute or more
//! opening the file in libVLC for each frame.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;

/// Shared cancellation token that immediately terminates any active FFmpeg
/// child processes registered on it so preview passes stop and reap promptly.
#[derive(Default)]
pub struct CancelToken {
    cancelled: AtomicBool,
    children: Mutex<Vec<Arc<Mutex<Option<Child>>>>>,
}

impl CancelToken {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Marks the token cancelled and immediately kills every registered child
    /// process. The owning worker still reaps each process via `Child::wait`.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        let children: Vec<Arc<Mutex<Option<Child>>>> = {
            let mut guard = self.children.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *guard)
        };
        for slot in children {
            if let Some(child) = slot.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                let _ = child.kill();
            }
        }
    }

    fn register(&self, slot: Arc<Mutex<Option<Child>>>) -> bool {
        let mut guard = self.children.lock().unwrap_or_else(|e| e.into_inner());
        if self.cancelled.load(Ordering::Acquire) {
            if let Some(child) = slot.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                let _ = child.kill();
            }
            return false;
        }
        guard.push(slot);
        true
    }

    fn unregister(&self, slot: &Arc<Mutex<Option<Child>>>) {
        let mut guard = self.children.lock().unwrap_or_else(|e| e.into_inner());
        guard.retain(|s| !Arc::ptr_eq(s, slot));
    }
}

/// FFmpeg: the app's own, next to its program, or else one on the PATH.
/// Looked for once. (CI builds have an empty stand-in, which doesn't count.)
pub fn ffmpeg() -> Option<PathBuf> {
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
            let usable = |p: &PathBuf| p.metadata().is_ok_and(|m| m.is_file() && m.len() > 0);
            let own = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join(name))).filter(usable);
            own.or_else(|| std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(name)).find(usable))
        })
        .clone()
}

/// How FFmpeg decodes.
#[derive(Clone, Debug)]
pub enum Accel {
    /// NVIDIA's own decoder (NVDEC), frames copied back to be scaled.
    Cuda,
    /// VA-API on this render node (Linux), scaling on the GPU too.
    Vaapi(String),
    /// VideoToolbox (macOS) or D3D11VA (Windows), frames copied back to be
    /// scaled. Where the GPU can't decode a file, FFmpeg uses the CPU.
    Copy(&'static str),
    Cpu,
}

impl Accel {
    pub fn gpu(&self) -> bool {
        !matches!(self, Accel::Cpu)
    }

    fn args(&self) -> Vec<&str> {
        match self {
            // Frames stay on the GPU until `filter` downloads them, so a GPU
            // that can't decode the file fails rather than quietly using the CPU.
            Accel::Cuda => vec!["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"],
            Accel::Vaapi(d) => vec!["-hwaccel", "vaapi", "-hwaccel_device", d, "-hwaccel_output_format", "vaapi"],
            Accel::Copy(api) => vec!["-hwaccel", api, "-threads", "2"],
            Accel::Cpu => vec!["-threads", "2"],
        }
    }

    /// The filter that scales frames to `width` and logs each one's time.
    fn filter(&self, width: u32) -> String {
        match self {
            // 8-bit video downloads as NV12, 10-bit as P010.
            Accel::Cuda => format!("hwdownload,format=nv12|p010le,scale={width}:-2,showinfo"),
            Accel::Vaapi(_) => format!("scale_vaapi=w={width}:h=-2,hwdownload,format=nv12,showinfo"),
            _ => format!("scale={width}:-2,showinfo"),
        }
    }
}

/// How to decode video track `track` of `video`, dedicated GPU first: NVIDIA's
/// NVDEC (on Windows, or on Linux with NVIDIA's driver), then on Windows
/// D3D11VA on the default GPU, or on Linux each VA-API GPU, dedicated before
/// integrated, then the CPU. Each is tried on one keyframe first: a PC
/// without the GPU decoder fails every pass with it. On macOS, VideoToolbox
/// picks the GPU itself.
#[cfg(test)]
pub fn accel(ffmpeg: &Path, video: &str, track: usize, width: u32) -> Accel {
    let cancel = CancelToken::new();
    accel_cancelable(ffmpeg, video, track, width, &cancel)
}

/// Like [`accel`], registering each probe child on `cancel` so switching videos
/// kills slow GPU probes immediately instead of holding up the next run.
pub fn accel_cancelable(ffmpeg: &Path, video: &str, track: usize, width: u32, cancel: &CancelToken) -> Accel {
    if cancel.is_cancelled() {
        return Accel::Cpu;
    }
    if cfg!(target_os = "macos") {
        return Accel::Copy("videotoolbox");
    }
    let nvidia = cfg!(windows) || nvidia_driver();
    let nodes = if cfg!(windows) { Vec::new() } else { vaapi_nodes() };
    first_working(candidates(cfg!(windows), nvidia, nodes), |a| {
        !cancel.is_cancelled() && decodes_cancelable(ffmpeg, video, track, a, width, cancel)
    })
}

/// The GPU decoders to try, in order (see [`accel_cancelable`]).
fn candidates(windows: bool, nvidia: bool, vaapi: Vec<String>) -> Vec<Accel> {
    let mut list = Vec::new();
    if nvidia {
        list.push(Accel::Cuda);
    }
    if windows {
        list.push(Accel::Copy("d3d11va"));
    }
    list.extend(vaapi.into_iter().map(Accel::Vaapi));
    list
}

/// The first of `candidates` that `works`, or else the CPU.
fn first_working(candidates: Vec<Accel>, works: impl FnMut(&Accel) -> bool) -> Accel {
    candidates.into_iter().find(works).unwrap_or(Accel::Cpu)
}

/// Whether NVIDIA's own driver is loaded (Linux). Without it (nouveau, say),
/// NVDEC can't work and isn't worth trying.
fn nvidia_driver() -> bool {
    Path::new("/proc/driver/nvidia/version").exists()
}

#[cfg(test)]
fn decodes(ffmpeg: &Path, video: &str, track: usize, accel: &Accel, width: u32) -> bool {
    decodes_cancelable(ffmpeg, video, track, accel, width, &CancelToken::new())
}

/// Whether FFmpeg can decode a keyframe of video track `track` of `video`
/// with `accel`, aborting immediately if `cancel` fires.
fn decodes_cancelable(ffmpeg: &Path, video: &str, track: usize, accel: &Accel, width: u32, cancel: &CancelToken) -> bool {
    if cancel.is_cancelled() {
        return false;
    }
    let Ok(child) = command(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
        .args(accel.args())
        .args(["-skip_frame", "nokey", "-i", video, "-map", &video_stream(track), "-frames:v", "1", "-vf", &accel.filter(width), "-f", "null", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let slot = Arc::new(Mutex::new(Some(child)));
    let registered = cancel.register(Arc::clone(&slot));
    if !registered {
        let mut c = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(mut c) = c.take() {
            let _ = c.wait();
        }
        return false;
    }
    // Wait without holding the slot lock so `cancel()` can kill the child while blocked?
    // Actually `Child::wait` requires `&mut Child`; instead we poll briefly or wait on a helper
    // — wait: on a fast 1-frame probe, wait via try_wait with 5ms sleep so `cancel()` can always
    // acquire `slot` and call `child.kill()` immediately without blocking!
    let status = loop {
        {
            let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
            let Some(child) = guard.as_mut() else {
                break None;
            };
            if cancel.is_cancelled() {
                let _ = child.kill();
                let res = child.wait().ok();
                *guard = None;
                break res;
            }
            match child.try_wait() {
                Ok(Some(st)) => {
                    *guard = None;
                    break Some(st);
                }
                Ok(None) => {}
                Err(_) => {
                    *guard = None;
                    break None;
                }
            }
        }
        thread::sleep(std::time::Duration::from_millis(5));
    };
    cancel.unregister(&slot);
    !cancel.is_cancelled() && status.is_some_and(|s| s.success())
}

/// FFmpeg's name for the `track`th video stream, not counting cover art.
fn video_stream(track: usize) -> String {
    format!("0:V:{track}")
}

/// An FFmpeg command. On Windows, without a console window flashing up, and
/// below normal priority so it gives way to playback (elsewhere, see
/// [`lower_priority`]).
fn command(ffmpeg: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        cmd.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    }
    cmd
}

/// The VA-API render nodes to try (Linux), dedicated GPUs first.
fn vaapi_nodes() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/dev/dri") else { return Vec::new() };
    let nodes = entries.flatten().filter_map(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("renderD") {
            return None;
        }
        let device = Path::new("/sys/class/drm").join(&name).join("device");
        let read = |file: &str| std::fs::read_to_string(device.join(file)).ok().map(|s| s.trim().to_string());
        let vendor = read("vendor").and_then(|v| u16::from_str_radix(v.trim_start_matches("0x"), 16).ok()).unwrap_or(0);
        let vram = read("mem_info_vram_total").and_then(|v| v.parse().ok());
        Some((e.path().to_string_lossy().into_owned(), vendor, vram))
    });
    vaapi_order(nodes.collect())
}

/// Orders render nodes, given as (path, PCI vendor, video memory in bytes),
/// dedicated GPUs first. NVIDIA's are left out: VA-API doesn't drive them
/// (NVDEC does, with NVIDIA's driver), and nouveau fails slowly. A GPU with
/// at least 2 GiB of its own memory counts as dedicated; integrated ones have
/// a smaller share of system memory, or report none.
fn vaapi_order(mut nodes: Vec<(String, u16, Option<u64>)>) -> Vec<String> {
    const NVIDIA: u16 = 0x10de;
    const DEDICATED: u64 = 2 << 30;
    nodes.retain(|n| n.1 != NVIDIA);
    nodes.sort_by_key(|(path, _, vram)| (vram.is_none_or(|v| v < DEDICATED), path.clone()));
    nodes.into_iter().map(|n| n.0).collect()
}

/// Decodes the keyframes of video track `track` of `video` from `from` ms
/// for `len` ms, `width` pixels wide, calling `frame` with each one's time in
/// ms and its JPEG. Stops early when `frame` returns false.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn pass(
    ffmpeg: &Path,
    video: &str,
    track: usize,
    accel: &Accel,
    from: i64,
    len: i64,
    width: u32,
    frame: impl FnMut(i64, Vec<u8>) -> bool,
) -> Result<(), String> {
    let cancel = CancelToken::new();
    pass_cancelable(ffmpeg, video, track, accel, from, len, width, &cancel, frame)
}

/// Like [`pass`], with an explicit [`CancelToken`] that kills the FFmpeg child
/// immediately on cancellation instead of waiting for the next frame.
#[allow(clippy::too_many_arguments)]
pub fn pass_cancelable(
    ffmpeg: &Path,
    video: &str,
    track: usize,
    accel: &Accel,
    from: i64,
    len: i64,
    width: u32,
    cancel: &CancelToken,
    mut frame: impl FnMut(i64, Vec<u8>) -> bool,
) -> Result<(), String> {
    if cancel.is_cancelled() {
        return Ok(());
    }
    let secs = |ms: i64| format!("{:.3}", ms as f64 / 1000.0);
    let mut child = command(ffmpeg)
        // `info`, for showinfo's line per frame.
        .args(["-hide_banner", "-nostats", "-nostdin", "-loglevel", "info"])
        .args(accel.args())
        // Keyframes only; times as in the file, not from the seek point.
        .args(["-skip_frame", "nokey", "-copyts", "-ss", &secs(from), "-t", &secs(len), "-i", video])
        .args(["-map", &video_stream(track), "-vf", &accel.filter(width), "-fps_mode", "passthrough"])
        .args(["-f", "image2pipe", "-c:v", "mjpeg", "-q:v", "5", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    lower_priority(child.id());
    let (Some(mut out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("no pipes to ffmpeg".into());
    };
    let slot = Arc::new(Mutex::new(Some(child)));
    let registered = cancel.register(Arc::clone(&slot));
    // Frame times come on stderr, in the same order as the JPEGs on stdout.
    let (tx, times) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            if let Some(ms) = pts_ms(&line)
                && tx.send(ms).is_err()
            {
                return;
            }
        }
    });
    let (mut buf, mut chunk, mut stopped, mut failed) = (Vec::new(), vec![0u8; 1 << 16], !registered, None);
    if !stopped {
        'read: loop {
            if cancel.is_cancelled() {
                stopped = true;
                break;
            }
            let n = match out.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    if cancel.is_cancelled() {
                        stopped = true;
                    } else {
                        failed = Some(e.to_string());
                    }
                    break;
                }
            };
            buf.extend_from_slice(&chunk[..n]);
            while let Some(jpeg) = take_jpeg(&mut buf) {
                if cancel.is_cancelled() {
                    stopped = true;
                    break 'read;
                }
                let Ok(at) = times.recv() else { break 'read };
                if !frame(at, jpeg) {
                    stopped = true;
                    break 'read;
                }
            }
        }
    }
    cancel.unregister(&slot);
    // Take ownership of `Child` before `wait()` so `cancel()` can never race
    // with reaping or kill a recycled PID.
    let mut child = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(c) = child.as_mut()
        && (stopped || failed.is_some() || cancel.is_cancelled())
    {
        let _ = c.kill();
    }
    drop(out);
    let status = match child {
        Some(mut c) => c.wait().map_err(|e| e.to_string())?,
        None => return Ok(()),
    };
    let _ = reader.join();
    match failed {
        Some(e) => Err(e),
        None if stopped || cancel.is_cancelled() || status.success() => Ok(()),
        None => Err(format!("ffmpeg: {status}")),
    }
}

/// Lowers a process's (on Linux, also a thread's) priority, so preview work
/// gives way to playback. Windows does it as the process starts instead.
pub fn lower_priority(id: u32) {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    // SAFETY: setpriority on a process or thread of ours.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS as _, id as libc::id_t, 10);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = id;
}

/// The frame time in a showinfo line ("... pts_time:12.345 ..."), in ms.
fn pts_ms(line: &str) -> Option<i64> {
    let rest = &line[line.find("pts_time:")? + "pts_time:".len()..];
    let secs: f64 = rest.split_whitespace().next()?.parse().ok()?;
    Some((secs * 1000.0).round() as i64)
}

/// Takes the first whole JPEG off the front of `buf`, if there is one: from
/// its start marker, through its marker segments and image data, to its end
/// marker. A plain search for the end marker could be fooled by table bytes.
fn take_jpeg(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    let start = buf.windows(2).position(|w| w == [0xFF, 0xD8])?;
    let mut i = start + 2;
    let end = loop {
        if i + 2 > buf.len() {
            return None;
        }
        if buf[i] != 0xFF {
            // Not a marker: damaged; skip past this start marker.
            buf.drain(..start + 2);
            return None;
        }
        match buf[i + 1] {
            0xFF => i += 1,
            0xD9 => break i + 2,
            0x01 | 0xD0..=0xD7 => i += 2,
            marker => {
                if i + 4 > buf.len() {
                    return None;
                }
                i += 2 + u16::from_be_bytes([buf[i + 2], buf[i + 3]]) as usize;
                // After the scan header comes image data, up to the next
                // marker that isn't an escaped 0xFF or a restart.
                if marker == 0xDA {
                    loop {
                        if i + 2 > buf.len() {
                            return None;
                        }
                        if buf[i] == 0xFF && buf[i + 1] != 0 && !(0xD0..=0xD7).contains(&buf[i + 1]) {
                            break;
                        }
                        i += 1;
                    }
                }
            }
        }
    };
    let jpeg = buf[start..end].to_vec();
    buf.drain(..end);
    Some(jpeg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(shade: u8) -> Vec<u8> {
        let img = image::RgbImage::from_fn(48, 27, |x, y| image::Rgb([shade, (x * 5) as u8, (y * 9) as u8]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    fn names(list: &[Accel]) -> Vec<String> {
        list.iter().map(|a| format!("{a:?}")).collect()
    }

    #[test]
    fn windows_tries_nvdec_then_d3d11va() {
        assert_eq!(names(&candidates(true, true, Vec::new())), ["Cuda", "Copy(\"d3d11va\")"]);
    }

    #[test]
    fn linux_tries_nvdec_then_each_vaapi_gpu() {
        let nodes = vec!["/dev/dri/renderD129".to_string(), "/dev/dri/renderD128".to_string()];
        assert_eq!(names(&candidates(false, true, nodes)), ["Cuda", "Vaapi(\"/dev/dri/renderD129\")", "Vaapi(\"/dev/dri/renderD128\")"]);
        assert!(candidates(false, false, Vec::new()).is_empty());
    }

    #[test]
    fn a_gpu_that_cant_decode_falls_back() {
        let windows = || candidates(true, true, Vec::new());
        // No NVIDIA GPU: D3D11VA.
        assert_eq!(format!("{:?}", first_working(windows(), |a| !matches!(a, Accel::Cuda))), "Copy(\"d3d11va\")");
        // No GPU decoding at all: the CPU, after trying each in order.
        let mut tried = Vec::new();
        let got = first_working(windows(), |a| {
            tried.push(format!("{a:?}"));
            false
        });
        assert!(matches!(got, Accel::Cpu));
        assert_eq!(tried, ["Cuda", "Copy(\"d3d11va\")"]);
        // NVDEC that works is used without trying the rest.
        assert!(matches!(first_working(windows(), |_| true), Accel::Cuda));
    }

    #[test]
    fn jpegs_come_off_a_stream_whole() {
        let (a, b) = (jpeg(10), jpeg(200));
        let stream: Vec<u8> = a.iter().chain(&b).copied().collect();
        // Fed in small pieces, as from a pipe.
        let (mut buf, mut got) = (Vec::new(), Vec::new());
        for piece in stream.chunks(7) {
            buf.extend_from_slice(piece);
            while let Some(j) = take_jpeg(&mut buf) {
                got.push(j);
            }
        }
        assert_eq!(got, vec![a, b]);
        assert!(buf.is_empty());
    }

    #[test]
    fn dedicated_gpus_first_nvidia_left_to_nvdec() {
        let node = |p: &str, vendor, vram| (p.to_string(), vendor, vram);
        // This laptop: an AMD APU (512 MiB carve-out) and an NVIDIA card.
        assert_eq!(vaapi_order(vec![node("/dev/dri/renderD129", 0x10de, None), node("/dev/dri/renderD128", 0x1002, Some(512 << 20))]), ["/dev/dri/renderD128"]);
        // A desktop: Intel integrated, then an AMD card with 8 GiB.
        let order = vaapi_order(vec![node("/dev/dri/renderD128", 0x8086, None), node("/dev/dri/renderD129", 0x1002, Some(8 << 30))]);
        assert_eq!(order, ["/dev/dri/renderD129", "/dev/dri/renderD128"]);
    }

    #[test]
    fn frame_times_from_showinfo() {
        let line = "[Parsed_showinfo_1 @ 0x5d] n:   3 pts:  31250 pts_time:31.25   duration:   1001 fmt:nv12";
        assert_eq!(pts_ms(line), Some(31_250));
        assert_eq!(pts_ms("frame=  10 fps=0.0"), None);
    }

    #[cfg(unix)]
    #[test]
    fn cancel_token_kills_and_reaps_child_promptly() {
        let token = CancelToken::new();
        let child = Command::new("/bin/sleep").arg("30").stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let slot = Arc::new(Mutex::new(Some(child)));
        assert!(token.register(Arc::clone(&slot)));
        let started = std::time::Instant::now();
        token.cancel();
        let mut child = slot.lock().unwrap().take().unwrap();
        let status = child.wait().unwrap();
        assert!(!status.success(), "killed child must not succeed");
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());
        // Registering after cancel immediately kills the child.
        let child2 = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let slot2 = Arc::new(Mutex::new(Some(child2)));
        assert!(!token.register(Arc::clone(&slot2)));
        let mut child2 = slot2.lock().unwrap().take().unwrap();
        let _ = child2.wait();
    }

    #[test]
    fn accel_cancelable_returns_cpu_immediately_when_already_cancelled() {
        let token = CancelToken::new();
        token.cancel();
        let picked = accel_cancelable(Path::new("nonexistent-ffmpeg"), "video.mp4", 0, 320, &token);
        assert!(matches!(picked, Accel::Cpu));
    }
}

#[cfg(test)]
#[path = "keyframes_it.rs"]
mod keyframes_it;
