#!/usr/bin/env bash
# Build the host binaries for aarch64-linux-android (release).
# Produces:
#   wandr-host/target/aarch64-linux-android/release/wasm-android-host
#   wandr-arbiter/target/aarch64-linux-android/release/wandr-arbiter
# For the device-default APK flow, use scripts/build-apk.sh instead.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=./env-android.sh
source "$REPO_ROOT/tools/scripts/env-android.sh"

cd "$REPO_ROOT/runtime/wandr-host"
echo "Building wandr-host for aarch64-linux-android …"
cargo build --target aarch64-linux-android --release

HOST_OUT="$REPO_ROOT/runtime/wandr-host/target/aarch64-linux-android/release/wasm-android-host"
echo "Built: $(du -sh "$HOST_OUT" | cut -f1)  $HOST_OUT"

# Task 46 step 3 — wandr-arbiter is a separate cargo crate, deliberately
# tiny (no wasmtime/skia/libgui). Same NDK/sysroot config — see
# wandr-arbiter/.cargo/config.toml which mirrors wandr-host's.
cd "$REPO_ROOT/runtime/wandr-arbiter"
echo "Building wandr-arbiter for aarch64-linux-android …"
cargo build --target aarch64-linux-android --release

ARB_OUT="$REPO_ROOT/runtime/wandr-arbiter/target/aarch64-linux-android/release/wandr-arbiter"
echo "Built: $(du -sh "$ARB_OUT" | cut -f1)  $ARB_OUT"
