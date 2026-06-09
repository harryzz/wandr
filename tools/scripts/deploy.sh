#!/usr/bin/env bash
# Push host binary + WASM component to device and run.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DEVICE_DIR="/data/local/tmp/wasm-runtime"
HOST_BIN="$REPO_ROOT/runtime/wandr-host/target/aarch64-linux-android/release/wasm-android-host"
WASM_FILE="${1:-$REPO_ROOT/compose-app.cwasm}"

if [[ ! -f "$HOST_BIN" ]]; then
    echo "Host binary not found — run scripts/build-host-android.sh first"
    exit 1
fi
if [[ ! -f "$WASM_FILE" ]]; then
    echo "WASM file not found: $WASM_FILE"
    echo "Run scripts/build-aot.sh first to produce compose-app.cwasm"
    exit 1
fi

echo "Pushing files to device …"
adb shell mkdir -p "$DEVICE_DIR"
adb push "$HOST_BIN"  "$DEVICE_DIR/wasm-host"
adb push "$WASM_FILE" "$DEVICE_DIR/$(basename "$WASM_FILE")"
adb shell chmod +x "$DEVICE_DIR/wasm-host"

echo "Launching on device …"
adb shell su -c "$DEVICE_DIR/wasm-host $DEVICE_DIR/$(basename "$WASM_FILE")" &
ADB_PID=$!

echo "Streaming logcat (Ctrl-C to stop) …"
adb logcat -s "RustStdoutStderr:*" "WASM:*" "*:E" | grep --line-buffered -E "android_main|EGL|WASM|render|error|Error" &
LOGCAT_PID=$!

wait $ADB_PID
kill $LOGCAT_PID 2>/dev/null || true
