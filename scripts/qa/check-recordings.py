#!/usr/bin/env python3
"""Checks a QA run's recordings and makes small MP4 copies of the .mov ones.

    python3 -I scripts/qa/check-recordings.py RUN_DIR

Every clip must have a duration and no audio stream (QA runs are silent).
.mov clips (macOS screencapture) get a 1080p-max H.264 .mp4 beside them, two
at a time. Writes RUN_DIR/logs/recording-check.json; exits 1 on a bad clip.
"""
import concurrent.futures
import json
import shutil
import subprocess
import sys
from pathlib import Path

run = Path(sys.argv[1]).expanduser()
rec = run / "recordings"
ffmpeg, ffprobe = shutil.which("ffmpeg"), shutil.which("ffprobe")


def encode(mov):
    mp4 = mov.with_suffix(".mp4")
    if not mp4.exists():
        subprocess.run([ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-i", str(mov), "-vf", "scale=min(1920\\,iw):-2",
                        "-c:v", "libx264", "-preset", "veryfast", "-crf", "25", "-pix_fmt", "yuv420p", "-an",
                        "-movflags", "+faststart", str(mp4)], check=True)


with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
    list(pool.map(encode, sorted(rec.glob("*.mov"))))

results, bad = [], False
for clip in sorted(rec.glob("*.mp4")):
    out = subprocess.run([ffprobe, "-v", "error", "-show_entries", "format=duration:stream=codec_type", "-of", "json", str(clip)],
                         capture_output=True, text=True)
    info = json.loads(out.stdout or "{}")
    duration = float(info.get("format", {}).get("duration", 0))
    audio = any(s.get("codec_type") == "audio" for s in info.get("streams", []))
    ok = out.returncode == 0 and duration > 1 and not audio
    bad |= not ok
    results.append({"file": clip.name, "seconds": round(duration, 1), "audio": audio, "ok": ok})
    print(f"{'ok ' if ok else 'BAD'} {clip.name} {duration:.1f}s{' AUDIO' if audio else ''}")

(run / "logs").mkdir(exist_ok=True)
(run / "logs/recording-check.json").write_text(json.dumps(results, indent=2))
sys.exit(1 if bad else 0)
