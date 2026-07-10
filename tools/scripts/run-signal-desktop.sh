#!/usr/bin/env bash
# Launch wandr.signal on the desktop (Linux / WSLg).
#
# Prereqs (already done if you built this session):
#   - Signal installed into the desktop apps root:
#       WANDR_APPS_ROOT=~/wandr-desktop-apps <host> --install <signal.wandrpkg>
#     and linked once (scan the QR from your phone: Settings → Linked Devices).
#   - The host built with --features p3-async (Signal rides native CM-async wasi
#     0.3). NOTE: the default and p3-async flavors build to the SAME binary path,
#     so a plain `cargo build` silently clobbers the p3 one. If launch fails with
#     "resource implementation is missing", rebuild:
#       (cd runtime/wandr-host && cargo build --release \
#          --target x86_64-unknown-linux-gnu --features p3-async)
#
# Env knobs (all overridable):
#   WANDR_HOST         path to the wasm-android-host binary
#   WANDR_APPS_ROOT    desktop apps sandbox (holds Signal + its linked /state)
#   WANDR_DESKTOP_SIZE window size (phone-shaped default)
set -euo pipefail

HOST="${WANDR_HOST:-$HOME/wandr/runtime/wandr-host/target/x86_64-unknown-linux-gnu/release/wasm-android-host}"
APPS_ROOT="${WANDR_APPS_ROOT:-$HOME/wandr-desktop-apps}"
SIZE="${WANDR_DESKTOP_SIZE:-520x1040}"

[ -x "$HOST" ] || { echo "host binary not found/executable: $HOST" >&2; exit 1; }

# -u WAYLAND_DISPLAY      → force X11 (the WSL Wayland compositor is flaky here).
# WINIT_X11_SCALE_FACTOR=1 → 1:1 display scale: stops the fractional-scale window
#                            runaway AND gives the UI its correct (non-doubled) size.
exec env -u WAYLAND_DISPLAY \
  WINIT_X11_SCALE_FACTOR=1 \
  WANDR_APPS_ROOT="$APPS_ROOT" \
  WANDR_DESKTOP_SIZE="$SIZE" \
  RUST_LOG="${RUST_LOG:-info}" \
  "$HOST" --app wandr.signal
