#!/usr/bin/env bash
# Experimental Mint 22 / Ubuntu 24.04 .deb. Uses distro libVLC; no release,
# updater key, bundled VLC download, or publishing step.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
[[ "$(uname -s)" == Linux ]] || { echo "Build this package on Linux (Mint 22 / Ubuntu 24.04)." >&2; exit 1; }
cd "$ROOT"
npx tauri build --bundles deb "$@"
