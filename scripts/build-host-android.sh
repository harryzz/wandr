#!/usr/bin/env bash
# Build the host binary for aarch64-linux-android (release).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NDK_HOME="${ANDROID_NDK_HOME:-/home/harry/android-ndk-r27d}"
TOOLCHAIN="$NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"

export PATH="$TOOLCHAIN:$PATH"

cd "$REPO_ROOT/wart-host"
echo "Building host for aarch64-linux-android …"
cargo build --target aarch64-linux-android --release

OUT="$REPO_ROOT/wart-host/target/aarch64-linux-android/release/wasm-android-host"
echo "Built: $(du -sh "$OUT" | cut -f1)  $OUT"
