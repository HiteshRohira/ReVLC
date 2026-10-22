#!/usr/bin/env python3
"""Makes the macOS QA fixture set; existing files are kept, so re-running only
adds what's missing. See docs/audit-playbook.md (Windows: windows/generate-media.ps1).

    python3 -I scripts/qa/generate-media.py [--entry-cap]

Fixtures go to $QA_MEDIA (default ~/revlc-qa/media). --entry-cap also makes
the 200k-entry scan-cap folder (slow; only when the scan cap is under test).
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

media = Path(os.environ.get("QA_MEDIA", Path.home() / "revlc-qa/media")).expanduser()
ffmpeg = os.environ.get("QA_FFMPEG") or shutil.which("ffmpeg") or sys.exit("ffmpeg not found")


def ff(out, args):
    """Runs ffmpeg in the output's folder: fMP4 HLS writes init.mp4 relative
    to the working directory, which once landed in the repo."""
    target = media / out
    if target.exists():
        return
    target.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([ffmpeg, "-hide_banner", "-loglevel", "error", "-y", *args, str(target)], cwd=target.parent, check=True)


media.mkdir(parents=True, exist_ok=True)
src = str(media / "long-h264.mp4")
ff("long-h264.mp4", ["-f", "lavfi", "-i", "testsrc2=size=960x540:rate=24", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000",
                     "-t", "180", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "26", "-g", "48", "-c:a", "aac",
                     "-metadata", "title=QA Long H264 180 seconds"])
# Blue/red pair for replaced-file checks: different colour, length and title.
ff("replace-in-place.mp4", ["-f", "lavfi", "-i", "color=c=blue:size=640x360:rate=24", "-t", "48", "-c:v", "libx264",
                            "-preset", "ultrafast", "-metadata", "title=QA Blue Replacement A"])
ff("replacement-source.mp4", ["-f", "lavfi", "-i", "color=c=red:size=640x360:rate=24", "-t", "72", "-c:v", "libx264",
                              "-preset", "ultrafast", "-metadata", "title=QA Red Replacement B"])
ff("日本語 🦀 café — VP9.webm", ["-i", src, "-t", "25", "-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-b:v", "500k", "-an"])
ff("mpeg4-avi.avi", ["-i", src, "-t", "25", "-c:v", "mpeg4", "-q:v", "6", "-c:a", "mp3"])

srt = media / "paired-captions.srt"
if not srt.exists():
    srt.write_text("1\n00:00:00,000 --> 00:00:08,000\nQA SUBTITLE: English + 日本語 + café\n\n"
                   "2\n00:00:08,000 --> 00:00:18,000\nSecond subtitle: paired open test\n\n"
                   "3\n00:00:18,000 --> 00:02:50,000\nQA subtitle visible for seek and resume\n")
ff("multi-track.mkv", ["-i", src, "-i", str(srt), "-t", "45", "-map", "0:v", "-map", "0:a", "-map", "0:a", "-map", "1", "-c", "copy",
                       "-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "language=fra"])
# Video + subtitle with unrelated names, for opening/dropping them together.
paired = media / "paired-open"
paired.mkdir(exist_ok=True)
if not (paired / "z-video.mp4").exists():
    shutil.copy2(src, paired / "z-video.mp4")
if not (paired / "a-captions.srt").exists():
    shutil.copy2(srt, paired / "a-captions.srt")

for kind, ext in [("fmp4", "m4s"), ("mpegts", "ts")]:
    ff(f"hls-{kind}/stream.m3u8", ["-i", src, "-t", "36", "-c", "copy", "-hls_time", "6", "-hls_list_size", "0",
                                   "-hls_segment_type", kind, "-hls_segment_filename", f"seg%03d.{ext}"])
    (media / f"hls-{kind}/master.m3u8").write_text("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=960x540\nstream.m3u8\n")

(media / "zero-byte.mp4").touch()
(media / "corrupt.mp4").write_bytes(b"not a video\x00" * 100)
(media / "unsupported.xyz").write_text("QA unsupported media")
long_name = ("Very long filename " + "Episode 01 café 日本語 — " * 7)[:236] + ".mp4"
if not (media / long_name).exists():
    shutil.copy2(media / "replace-in-place.mp4", media / long_name)

# 80 episodes plus a symlink back to the folder itself.
lib = media / "library-many"
lib.mkdir(exist_ok=True)
for i in range(80):
    ep = lib / f"QA Show S01E{i + 1:02d}.mp4"
    if not ep.exists():
        ep.hardlink_to(media / "replace-in-place.mp4")
if not (lib / "loop").exists():
    (lib / "loop").symlink_to(lib, target_is_directory=True)
(media / "qa-playlist.m3u").write_text("#EXTM3U\nlong-h264.mp4\nmulti-track.mkv\n日本語 🦀 café — VP9.webm\n")

if "--entry-cap" in sys.argv:
    # Only 000-before-cap.mp4 should appear when this folder is added as a library.
    cap = media / "scan-entry-cap"
    cap.mkdir(exist_ok=True)
    shutil.copy2(media / "replacement-source.mp4", cap / "000-before-cap.mp4")
    for i in range(200005):
        (cap / f"entry-{i:06d}.txt").touch()
    shutil.copy2(media / "replace-in-place.mp4", cap / "zzz-after-cap.mp4")

print(f"Fixtures ready in {media}: {sum(1 for _ in media.rglob('*'))} entries")
