#!/usr/bin/env bash
# Runs inside the Ubuntu guest (from new-vms.ps1, through guestcontrol as the
# VM's user) to add what the Cua suite needs: Pillow, ffmpeg, curl and Cua
# Driver. Python 3 and AT-SPI ship with Ubuntu desktop.
#
# Also switches GDM to Xorg: Cua drives Linux through X11 (CI runs on Xvfb),
# and Ubuntu 24.04 logs in to Wayland by default.
#
#   SUDO_PASS=... guest-tools-linux.sh OUT_DIR
set -euo pipefail
out="$1"
mkdir -p "$out"
exec > >(tee "$out/tools-transcript.txt") 2>&1
sudo_() { printf '%s\n' "$SUDO_PASS" | sudo -S -p '' "$@"; }
export DEBIAN_FRONTEND=noninteractive

before="$(dpkg-query -W -f '${Package}\n' | sort)"
sudo_ apt-get -o DPkg::Lock::Timeout=600 update
sudo_ apt-get -o DPkg::Lock::Timeout=600 install -y --no-install-recommends python3-pil ffmpeg curl
after="$(dpkg-query -W -f '${Package}\n' | sort)"

/bin/bash -c "$(curl -fsSL https://cua.ai/driver/install.sh)" -- --no-modify-path
"$HOME/.local/bin/cua-driver" --version

# Xorg session instead of Wayland, from the next login.
sudo_ sed -i 's/^#\?\s*WaylandEnable=.*/WaylandEnable=false/' /etc/gdm3/custom.conf
grep -q '^WaylandEnable=false' /etc/gdm3/custom.conf || sudo_ sed -i '/^\[daemon\]/a WaylandEnable=false' /etc/gdm3/custom.conf

{
  echo "Python: $(python3 --version)"
  echo "Pillow: $(python3 -c 'import PIL; print(PIL.__version__)')"
  echo "ffmpeg: $(ffmpeg -version | head -1)"
  echo "Cua Driver: $("$HOME/.local/bin/cua-driver" --version)"
  echo "GDM: WaylandEnable=false (Xorg session)"
  echo
  echo "Packages added:"
  comm -13 <(echo "$before") <(echo "$after") | sed 's/^/  /'
} > "$out/tools.txt"
echo ok > "$out/done.txt"
