#!/usr/bin/env python3
"""Serves a local HLS fixture and stalls one segment until a flag file is
removed, for "quit while buffering" checks. A fixed sleep was too short last
time (the stream recovered before the quit), so the hold is open-ended.

    python3 -I scripts/qa/slow-hls-server.py [DIR] [--port 18876] [--hold seg002]

Open http://127.0.0.1:18876/stream.m3u8 in ReVLC, wait for Buffering, quit,
then release with:  rm "$TMPDIR/revlc-qa-hold"  (Windows: %TEMP%\\revlc-qa-hold).
DIR defaults to $QA_MEDIA/hls-mpegts.
"""
import argparse
import http.server
import os
import tempfile
import time
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument("dir", nargs="?", default=Path(os.environ.get("QA_MEDIA", Path.home() / "revlc-qa/media")) / "hls-mpegts")
p.add_argument("--port", type=int, default=18876)
p.add_argument("--hold", default="seg002", help="hold requests whose path contains this")
args = p.parse_args()
flag = Path(tempfile.gettempdir()) / "revlc-qa-hold"
flag.touch()


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=str(args.dir), **kw)

    def do_GET(self):
        if args.hold in self.path:
            print(f"HOLD {self.path} at {time.time():.3f} until {flag} is removed", flush=True)
            while flag.exists():
                time.sleep(0.2)
            print(f"RELEASE {self.path} at {time.time():.3f}", flush=True)
        super().do_GET()


print(f"Serving {args.dir} on http://127.0.0.1:{args.port}/", flush=True)
http.server.ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()
