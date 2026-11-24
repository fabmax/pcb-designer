#!/usr/bin/env bash
# Sets up a fresh Air cloud environment (Ubuntu) to build, test and run PCB Designer:
# the Rust toolchain, the system libraries the CI uses (.github/actions/linux-deps),
# a headless X server for real-window screenshots, and a warm debug build.
# Safe to run again: every step skips what is already there.
set -euo pipefail

cd "$(dirname "$0")/../.."
REPO="$(pwd)"
SUDO=""
[ "$(id -u)" -ne 0 ] && SUDO="sudo"

# --- System packages ---------------------------------------------------------------
# Build tools, the window's libraries (X11, Wayland, xkbcommon), Mesa's software
# Vulkan/GL drivers (wgpu without a GPU), the GTK file dialog (rfd), ngspice for the
# simulation tests, Xvfb, and ImageMagick for scaling and cropping screenshots.
$SUDO apt-get update -qq
DEBIAN_FRONTEND=noninteractive $SUDO apt-get install -y -qq --no-install-recommends \
  build-essential pkg-config curl ca-certificates git \
  libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 libx11-dev libx11-xcb1 \
  libxcursor-dev libxi-dev libxrandr-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libgtk-3-dev libxdo-dev libssl-dev \
  mesa-vulkan-drivers libvulkan1 libgl1-mesa-dri libegl1 \
  ngspice libngspice0 \
  xvfb x11-utils imagemagick

# --- Rust ----------------------------------------------------------------------------
if [ ! -x "$HOME/.cargo/bin/rustup" ]; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
fi
# shellcheck source=/dev/null
source "$HOME/.cargo/env"
rustup component add rustfmt clippy

# --- Headless X server ---------------------------------------------------------------
# 3200x2000 fits the app's 1600x1000-point window at 2x.
export DISPLAY=:99
if ! xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
  nohup Xvfb "$DISPLAY" -screen 0 3200x2000x24 +extension GLX +render -nolisten tcp \
    >/tmp/xvfb.log 2>&1 &
  for _ in $(seq 20); do xdpyinfo -display "$DISPLAY" >/dev/null 2>&1 && break; sleep 0.5; done
fi

# Every later shell gets cargo and the display.
for rc in "$HOME/.bashrc" "$HOME/.profile"; do
  grep -q 'pcb-designer cloud env' "$rc" 2>/dev/null || cat >>"$rc" <<'EOF'
# pcb-designer cloud env
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
export DISPLAY="${DISPLAY:-:99}"
EOF
done

# --- Dependencies and a warm build -------------------------------------------------
# A cold debug build of the app takes 15+ minutes on 2 cores. Fetch now, build in the
# background so the session can start; a later cargo command waits for its lock.
cargo fetch --locked
nohup cargo build -p app >/tmp/startup-build.log 2>&1 &

echo "Setup done. DISPLAY=$DISPLAY; app build running in background (/tmp/startup-build.log)."
