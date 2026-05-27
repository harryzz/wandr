#!/usr/bin/env bash
# Build the host binaries for aarch64-linux-android (release).
# Produces:
#   wart-host/target/aarch64-linux-android/release/wasm-android-host
#   wart-arbiter/target/aarch64-linux-android/release/wart-arbiter
# For the device-default APK flow, use scripts/build-apk.sh instead.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=./env-android.sh
source "$REPO_ROOT/scripts/env-android.sh"

cd "$REPO_ROOT/wart-host"
echo "Building wart-host for aarch64-linux-android …"
cargo build --target aarch64-linux-android --release

HOST_OUT="$REPO_ROOT/wart-host/target/aarch64-linux-android/release/wasm-android-host"
echo "Built: $(du -sh "$HOST_OUT" | cut -f1)  $HOST_OUT"

# Task 46 step 3 — wart-arbiter is a separate cargo crate, deliberately
# tiny (no wasmtime/skia/libgui). Same NDK/sysroot config — see
# wart-arbiter/.cargo/config.toml which mirrors wart-host's.
cd "$REPO_ROOT/wart-arbiter"
echo "Building wart-arbiter for aarch64-linux-android …"
cargo build --target aarch64-linux-android --release

ARB_OUT="$REPO_ROOT/wart-arbiter/target/aarch64-linux-android/release/wart-arbiter"
echo "Built: $(du -sh "$ARB_OUT" | cut -f1)  $ARB_OUT"
