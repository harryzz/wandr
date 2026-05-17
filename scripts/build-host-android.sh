#!/usr/bin/env bash
# Build the host binary for aarch64-linux-android (release).
# Produces a bare binary at:
#   wart-host/target/aarch64-linux-android/release/wasm-android-host
# For the device-default APK flow, use scripts/build-apk.sh instead.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=./env-android.sh
source "$REPO_ROOT/scripts/env-android.sh"

cd "$REPO_ROOT/wart-host"
echo "Building host for aarch64-linux-android …"
cargo build --target aarch64-linux-android --release

OUT="$REPO_ROOT/wart-host/target/aarch64-linux-android/release/wasm-android-host"
echo "Built: $(du -sh "$OUT" | cut -f1)  $OUT"
