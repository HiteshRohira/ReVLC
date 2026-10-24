//! Seek bar preview frames from FFmpeg (a sidecar next to ReVLC on macOS and
//! Windows, a package dependency on Linux): one process decodes just the keyframes of a
//! stretch of video, on the GPU where it can (VA-API, VideoToolbox, D3D11VA),
//! and hands them over as JPEGs with their times. A 2-hour 4K HEVC film
//! takes about 11 s on a laptop's integrated GPU, against a minute or more
//! opening the file in libVLC for each frame.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{OnceLock, mpsc};
use std::thread;

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
            Accel::Vaapi(d) => vec!["-hwaccel", "vaapi", "-hwaccel_device", d, "-hwaccel_output_format", "vaapi"],
            Accel::Copy(api) => vec!["-hwaccel", api, "-threads", "2"],
            Accel::Cpu => vec!["-threads", "2"],
        }
    }

    /// The filter that scales frames to `width` and logs each one's time.
    fn filter(&self, width: u32) -> String {
        match self {
            Accel::Vaapi(_) => format!("scale_vaapi=w={width}:h=-2,hwdownload,format=nv12,showinfo"),
            _ => format!("scale={width}:-2,showinfo"),
        }
    }
}

/// How to decode `video`: on macOS and Windows, the system's video API; on
/// Linux, the first VA-API device that can decode it, if any.
pub fn accel(ffmpeg: &Path, video: &str, width: u32) -> Accel {
    if cfg!(target_os = "macos") {
        Accel::Copy("videotoolbox")
    } else if cfg!(windows) {
        Accel::Copy("d3d11va")
    } else {
        vaapi_device(ffmpeg, video, width).map_or(Accel::Cpu, Accel::Vaapi)
    }
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

/// A VA-API render node that can decode `video`, trying each in turn (a
/// laptop may have two GPUs, one without a working video decoder), the one
/// that worked last time first; `None` to decode on the CPU.
fn vaapi_device(ffmpeg: &Path, video: &str, width: u32) -> Option<String> {
    static LAST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    if !cfg!(target_os = "linux") {
        return None;
    }
    let last = LAST.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut nodes: Vec<PathBuf> = std::fs::read_dir("/dev/dri")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("renderD")))
        .collect();
    nodes.sort();
    let mut nodes: Vec<String> = nodes.into_iter().map(|n| n.to_string_lossy().into_owned()).collect();
    if let Some(last) = last {
        nodes.sort_by_key(|n| *n != last);
    }
    let found = nodes.into_iter().find(|node| {
        let accel = Accel::Vaapi(node.clone());
        command(ffmpeg)
            .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
            .args(accel.args())
            .args(["-skip_frame", "nokey", "-i", video, "-an", "-sn", "-dn", "-frames:v", "1", "-vf", &accel.filter(width), "-f", "null", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    });
    if found.is_some() {
        *LAST.lock().unwrap_or_else(|e| e.into_inner()) = found.clone();
    }
    found
}

/// Decodes the keyframes from `from` ms for `len` ms of `video`, `width`
/// pixels wide, calling `frame` with each one's time in ms and its JPEG.
/// Stops early when `frame` returns false.
pub fn pass(
    ffmpeg: &Path,
    video: &str,
    accel: &Accel,
    from: i64,
    len: i64,
    width: u32,
    mut frame: impl FnMut(i64, Vec<u8>) -> bool,
) -> Result<(), String> {
    let secs = |ms: i64| format!("{:.3}", ms as f64 / 1000.0);
    let mut child = command(ffmpeg)
        // `info`, for showinfo's line per frame.
        .args(["-hide_banner", "-nostats", "-nostdin", "-loglevel", "info"])
        .args(accel.args())
        // Keyframes only; times as in the file, not from the seek point.
        .args(["-skip_frame", "nokey", "-copyts", "-ss", &secs(from), "-t", &secs(len), "-i", video])
        .args(["-an", "-sn", "-dn", "-vf", &accel.filter(width), "-fps_mode", "passthrough"])
        .args(["-f", "image2pipe", "-c:v", "mjpeg", "-q:v", "5", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    lower_priority(child.id());
    let (Some(mut out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        return Err("no pipes to ffmpeg".into());
    };
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
    let (mut buf, mut chunk, mut stopped, mut failed) = (Vec::new(), vec![0u8; 1 << 16], false, None);
    'read: loop {
        let n = match out.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                failed = Some(e.to_string());
                break;
            }
        };
        buf.extend_from_slice(&chunk[..n]);
        while let Some(jpeg) = take_jpeg(&mut buf) {
            let Ok(at) = times.recv() else { break 'read };
            if !frame(at, jpeg) {
                stopped = true;
                break 'read;
            }
        }
    }
    if stopped || failed.is_some() {
        let _ = child.kill();
    }
    drop(out);
    let status = child.wait().map_err(|e| e.to_string())?;
    let _ = reader.join();
    match failed {
        Some(e) => Err(e),
        None if stopped || status.success() => Ok(()),
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
    fn frame_times_from_showinfo() {
        let line = "[Parsed_showinfo_1 @ 0x5d] n:   3 pts:  31250 pts_time:31.25   duration:   1001 fmt:nv12";
        assert_eq!(pts_ms(line), Some(31_250));
        assert_eq!(pts_ms("frame=  10 fps=0.0"), None);
    }
}
