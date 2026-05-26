#!/usr/bin/env bash
# Launch wart-host --standalone on the device with SystemUI + the
# launcher stopped so the runtime owns the screen. Restores both on
# exit (Ctrl-C, normal exit, wart-host crash) via an EXIT trap.
#
# Usage:
#   bash scripts/standalone-launch.sh [--shim <path>] [--cwasm <path>]
#
# Defaults (per Step 3 build flow + CLAUDE.md "Build pipeline"):
#   --shim   /tmp/libsf_surface.so
#   --cwasm  /tmp/skiko-component.cwasm
#
# Recovery: if this script dies before its trap fires, run
#   bash scripts/standalone-recover.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

SHIM="/tmp/libsf_surface.so"
CWASM="/tmp/skiko-component.cwasm"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --shim)  SHIM="$2";  shift 2 ;;
        --cwasm) CWASM="$2"; shift 2 ;;
        -h|--help)
            grep -E '^# ' "$0" | sed 's/^# \?//'
            exit 0
            ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

HOST_BIN="$REPO_ROOT/wart-host/target/aarch64-linux-android/release/wasm-android-host"

# ── pre-flight ───────────────────────────────────────────────────────

# adb on WSL pipes CRLF — strip \r before comparing.
adb_state="$(adb get-state 2>/dev/null | tr -d '\r' || true)"
if [[ "$adb_state" != "device" ]]; then
    echo "✗ no adb device (adb get-state = '$adb_state') — 'adb devices' to confirm." >&2
    exit 1
fi

if ! adb shell "su -c id" 2>/dev/null | tr -d '\r' | grep -q 'uid=0'; then
    echo "✗ root not available — 'su -c id' did not return uid=0." >&2
    exit 1
fi

if [[ ! -f "$HOST_BIN" ]]; then
    echo "host binary missing — building …"
    bash "$REPO_ROOT/scripts/build-host-android.sh"
fi

if [[ ! -f "$SHIM" ]]; then
    cat >&2 <<EOF
✗ libsf_surface.so missing at: $SHIM

Build it on the AOSP host (a-03) per tasks/33-boot-model-bringup.md:
  scp wart-host/cpp/sf_surface.cpp \\
      harry@a-03:~/android/lineage/external/sf_surface/sf_surface.cpp
  ssh harry@a-03 'cd ~/android/lineage && \\
      SO=out/soong/.intermediates/external/sf_surface/libsf_surface/android_arm64_armv8-a_shared/libsf_surface.so && \\
      prebuilts/build-tools/linux-x86/bin/ninja -f out/combined-aosp_arm64.ninja "\$SO"'
  scp harry@a-03:~/android/lineage/<that SO path> $SHIM
EOF
    exit 1
fi

if [[ ! -f "$CWASM" ]]; then
    cat >&2 <<EOF
✗ skiko-component.cwasm missing at: $CWASM

Build per CLAUDE.md "Build pipeline" (Kotlin → wasm → component → cwasm):
  cd $REPO_ROOT/wart-app
  ./gradlew compileProductionExecutableKotlinWasmWasi --console=plain --no-daemon

  wasm-tools component embed \\
      --world my:skiko-gfx/skiko-ui \\
      $REPO_ROOT/wit/skiko-gfx.wit \\
      build/compileSync/wasmWasi/main/productionExecutable/kotlin/wart-app.wasm \\
      -o /tmp/embedded.wasm

  wasm-tools component new /tmp/embedded.wasm \\
      --adapt $REPO_ROOT/wasmtime-src/target/wasm32-unknown-unknown/release/wasi_snapshot_preview1.wasm \\
      -o /tmp/skiko-component.wasm

  wasmtime compile --target aarch64-linux-android \\
      --wasm component-model --wasm gc --wasm function-references --wasm exceptions \\
      -o $CWASM /tmp/skiko-component.wasm
EOF
    exit 1
fi

# ── resolve home (launcher) package ─────────────────────────────────

HOME_PKG="$(
    adb shell "cmd package resolve-activity \
        -a android.intent.action.MAIN \
        -c android.intent.category.HOME" 2>/dev/null \
        | awk -F= '/packageName=/ { gsub(/[ \r]/, "", $2); print $2; exit }'
)"
if [[ -z "$HOME_PKG" ]]; then
    echo "✗ could not resolve home (launcher) package — bailing rather than" >&2
    echo "  flying blind. Run 'adb shell cmd package resolve-activity -c" >&2
    echo "  android.intent.category.HOME' and inspect." >&2
    exit 1
fi
echo "▸ home package: $HOME_PKG"

# ── restore function + EXIT trap ────────────────────────────────────

restore_ui() {
    # Idempotent — safe to run twice. Mirrors scripts/standalone-recover.sh.
    set +e
    adb shell "su -c 'pkill -9 -f wart-host'" >/dev/null 2>&1
    adb shell "su -c 'am start -n com.android.systemui/.SystemUIService'" >/dev/null 2>&1
    adb shell "input keyevent KEYCODE_HOME" >/dev/null 2>&1
    set -e
    echo "▸ SystemUI + launcher restored. If display still wedged: adb reboot."
}
trap restore_ui EXIT INT TERM

# ── kill old wart-host + push artifacts ─────────────────────────────

echo "▸ killing any existing wart-host …"
adb shell "su -c 'pkill -f wart-host'" >/dev/null 2>&1 || true

push_if_newer() {
    local local_path="$1" remote_path="$2"
    local remote_mtime local_mtime
    remote_mtime="$(
        adb shell "stat -c %Y '$remote_path' 2>/dev/null || echo 0" \
            | tr -d '\r'
    )"
    local_mtime="$(stat -c %Y "$local_path")"
    if (( local_mtime > remote_mtime )); then
        echo "  push  $local_path → $remote_path"
        adb push "$local_path" "$remote_path" >/dev/null
    else
        echo "  skip  $remote_path (device ≥ local)"
    fi
}

echo "▸ pushing artifacts …"
push_if_newer "$SHIM"     "/data/local/tmp/libsf_surface.so"
push_if_newer "$HOST_BIN" "/data/local/tmp/wart-host"
push_if_newer "$CWASM"    "/data/local/tmp/skiko-component.cwasm"
adb shell "su -c 'chmod 755 /data/local/tmp/wart-host'"

# ── stop SystemUI + launcher (non-persistent, am force-stop) ────────

echo "▸ stopping SystemUI + launcher ($HOME_PKG) …"
adb shell "su -c 'am force-stop com.android.systemui'"
adb shell "su -c 'am force-stop $HOME_PKG'"

# ── launch wart-host in the foreground ──────────────────────────────

echo "▸ launching wart-host --standalone (Ctrl-C to stop and restore UI)"
echo "──────────────────────────────────────────────────────────────────"
adb shell -t "su -c 'LD_LIBRARY_PATH=/data/local/tmp /data/local/tmp/wart-host --standalone'"
