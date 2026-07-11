#!/usr/bin/env bash
# Launch an installed wandr app on the macOS desktop backend.
#
# Usage:
#   run-app-macos.sh [app-id]        # default: wandr.audio.player
#
# Picks the release host for the Mac's own arch (arm64 → aarch64-apple-darwin,
# Intel → x86_64-apple-darwin); build it first with build-host-macos.sh. Apps
# must already be installed into WANDR_APPS_ROOT (`<host> --install <pkg-dir>`).
#
# Window size: a phone-shaped default (520x1040) AUTO-FITTED to the screen —
# a 1040-tall window overflows a 1280x800 laptop (the QR/link screen falls off
# the bottom). Set WANDR_DESKTOP_SIZE to override the auto-fit entirely.
#
# Env knobs (all overridable):
#   WANDR_HOST          path to the wasm-android-host binary
#   WANDR_APPS_ROOT     apps sandbox (default ~/wandr-apps)
#   WANDR_DESKTOP_SIZE  window size WxH (skips auto-fit when set)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APP="${1:-wandr.audio.player}"

case "$(uname -m)" in
  arm64)  TARGET=aarch64-apple-darwin ;;
  x86_64) TARGET=x86_64-apple-darwin ;;
  *) echo "run-app-macos.sh: unknown arch $(uname -m)" >&2; exit 1 ;;
esac

HOST="${WANDR_HOST:-$REPO_ROOT/runtime/wandr-host/target/$TARGET/release/wasm-android-host}"
APPS_ROOT="${WANDR_APPS_ROOT:-$HOME/wandr-apps}"

# Window size: honor an explicit WANDR_DESKTOP_SIZE, else auto-fit the default
# phone shape to the screen so it never runs off the bottom.
if [[ -n "${WANDR_DESKTOP_SIZE:-}" ]]; then
  SIZE="$WANDR_DESKTOP_SIZE"
else
  DEF_W=520; DEF_H=1040; W=$DEF_W; H=$DEF_H
  # Logical desktop bounds via Finder → "0, 0, <w>, <h>" (points, not pixels).
  bounds="$(osascript -e 'tell application "Finder" to get bounds of window of desktop' 2>/dev/null || true)"
  scr_h="$(printf '%s' "$bounds" | awk -F', ' '{print $4}')"
  if [[ "$scr_h" =~ ^[0-9]+$ ]] && (( scr_h > 0 )); then
    max_h=$(( scr_h * 85 / 100 ))          # leave room for menu bar + title + dock
    if (( DEF_H > max_h )); then
      H=$max_h
      W=$(( DEF_W * max_h / DEF_H ))        # keep the phone aspect ratio
    fi
  fi
  SIZE="${W}x${H}"
fi

if [[ ! -x "$HOST" ]]; then
  echo "host not built for $TARGET:" >&2
  echo "  $HOST" >&2
  echo "run tools/scripts/build-host-macos.sh first." >&2
  exit 1
fi

echo "launching $APP  (host: $TARGET, apps: $APPS_ROOT, size: $SIZE)"
exec env \
  WANDR_APPS_ROOT="$APPS_ROOT" \
  WANDR_DESKTOP_SIZE="$SIZE" \
  RUST_LOG="${RUST_LOG:-info}" \
  "$HOST" --app "$APP"
