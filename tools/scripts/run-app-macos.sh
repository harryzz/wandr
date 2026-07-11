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
# Env knobs (all overridable):
#   WANDR_HOST          path to the wasm-android-host binary
#   WANDR_APPS_ROOT     apps sandbox (default ~/wandr-apps)
#   WANDR_DESKTOP_SIZE  window size (phone-shaped default)
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
SIZE="${WANDR_DESKTOP_SIZE:-520x1040}"

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
