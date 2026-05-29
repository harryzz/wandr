#!/usr/bin/env bash
# Launch the Hybrid runtime stack on-device: wart-host --zygote + wart-arbiter
# --daemon, side by side, with SystemUI + the launcher stopped so the runtime
# owns the screen. Restores both on exit. Task 46 step 3.
#
# Usage:
#   scripts/run-hybrid-stack.sh
#
# Inputs (must already exist on the dev machine):
#   wart-host/target/aarch64-linux-android/release/wasm-android-host
#   wart-arbiter/target/aarch64-linux-android/release/wart-arbiter
#   wart-host/cpp/.../libsf_surface.so   (built via standalone-launch.sh helper)
#
# Once the stack is up, drive it from another shell:
#   adb shell '/data/local/tmp/wart-arbiter launch com.example.wart-app'
#   adb shell '/data/local/tmp/wart-arbiter list'
#   adb shell '/data/local/tmp/wart-arbiter kill com.example.wart-app'
#
# Stop the stack: Ctrl-C in this script's terminal.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

HOST_BIN="$REPO_ROOT/runtime/wart-host/target/aarch64-linux-android/release/wasm-android-host"
ARB_BIN="$REPO_ROOT/runtime/wart-arbiter/target/aarch64-linux-android/release/wart-arbiter"
SHIM="$REPO_ROOT/runtime/wart-host/cpp/build/libsf_surface.so"
APPS_ROOT="/data/local/tmp/wart-apps"
# Task 57 — the app the arbiter designates as "home": foregrounded at
# boot, on `go-home`, and as the fall-back when the foreground app dies.
# Set WART_HOME_APP="" to disable boot-to-launcher.
HOME_APP="${WART_HOME_APP-war.launcher}"

for bin in "$HOST_BIN" "$ARB_BIN" "$SHIM"; do
    if [[ ! -f "$bin" ]]; then
        echo "✗ missing: $bin" >&2
        echo "  Run scripts/build-host-android.sh and ensure libsf_surface.so" >&2
        echo "  is built via scripts/standalone-launch.sh's helper section." >&2
        exit 1
    fi
done

# Resolve home package the same way standalone-launch.sh does.
HOME_PKG="$(
    adb shell "cmd package resolve-activity \
        -a android.intent.action.MAIN \
        -c android.intent.category.HOME" 2>/dev/null \
        | awk -F= '/packageName=/ { gsub(/[ \r]/, "", $2); print $2; exit }'
)"
if [[ -z "$HOME_PKG" ]]; then
    echo "✗ could not resolve home (launcher) package — bailing." >&2
    exit 1
fi
echo "▸ home package: $HOME_PKG"

restore_ui() {
    set +e
    echo ""
    echo "▸ stopping wart-arbiter + wart-host …"
    adb shell "su -c 'pkill -9 -f wart-arbiter'" >/dev/null 2>&1
    adb shell "su -c 'pkill -9 -f wart-host'"    >/dev/null 2>&1
    adb shell "su -c 'rm -f /data/local/tmp/wart-zygote.sock /data/local/tmp/wart-arbiter.sock'" >/dev/null 2>&1
    adb shell "su -c 'am start -n com.android.systemui/.SystemUIService'" >/dev/null 2>&1
    adb shell "input keyevent KEYCODE_HOME" >/dev/null 2>&1
    set -e
    echo "▸ SystemUI + launcher restored. If display still wedged: adb reboot."
}
trap restore_ui EXIT INT TERM

push_if_newer() {
    local local_path="$1" remote_path="$2"
    local remote_mtime local_mtime
    remote_mtime="$(adb shell "stat -c %Y '$remote_path' 2>/dev/null || echo 0" | tr -d '\r')"
    local_mtime="$(stat -c %Y "$local_path")"
    if (( local_mtime > remote_mtime )); then
        echo "  push  $local_path → $remote_path"
        adb push "$local_path" "$remote_path" >/dev/null
    else
        echo "  skip  $remote_path (device ≥ local)"
    fi
}

echo "▸ killing any existing wart-host / wart-arbiter …"
adb shell "su -c 'pkill -9 -f wart-arbiter'" >/dev/null 2>&1 || true
adb shell "su -c 'pkill -9 -f wart-host'"    >/dev/null 2>&1 || true
adb shell "su -c 'rm -f /data/local/tmp/wart-zygote.sock /data/local/tmp/wart-arbiter.sock'" >/dev/null 2>&1

echo "▸ pushing artifacts …"
push_if_newer "$SHIM"     "/data/local/tmp/libsf_surface.so"
push_if_newer "$HOST_BIN" "/data/local/tmp/wart-host"
push_if_newer "$ARB_BIN"  "/data/local/tmp/wart-arbiter"
adb shell 'chmod 755 /data/local/tmp/wart-host /data/local/tmp/wart-arbiter'

echo "▸ stopping SystemUI + launcher ($HOME_PKG) …"
adb shell "su -c 'am force-stop com.android.systemui'"
adb shell "su -c 'am force-stop $HOME_PKG'"

echo "▸ starting wart-host --zygote in background …"
adb shell "su -c 'LD_LIBRARY_PATH=/data/local/tmp WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-host --zygote' &" >/dev/null
sleep 3
ZPID="$(adb shell 'pgrep -f "wart-host --zygote" | head -1' | tr -d '\r')"
echo "  zygote pid: $ZPID"

# Task 57 — boot straight to the launcher. The arbiter below runs in the
# foreground (this terminal), so we can't set-home after it. Instead a
# background waiter polls for the arbiter socket and, once it's up, sends
# `set-home $HOME_APP` — which designates the home app AND foregrounds it
# (launching it if needed). No manual `wart-arbiter launch` required.
# Once the arbiter socket is up, bring up the full shell: home/launcher
# (set-home), the top status bar (a direct top-overlay daemon), and the
# IME keyboard (bottom overlay + set-ime, so tapping an editor pops the
# keyboard). Each piece is best-effort — only fires if installed.
(
    for _ in $(seq 1 30); do
        if adb shell "su -c '[ -S /data/local/tmp/wart-arbiter.sock ] && echo up'" 2>/dev/null | grep -q up; then
            if [[ -n "$HOME_APP" ]]; then
                echo "▸ boot-to-home: set-home $HOME_APP"
                adb shell "su -c 'WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-arbiter set-home $HOME_APP'" 2>&1 | tr -d '\r'
            fi
            echo "▸ status bar (top overlay)"
            adb shell "su -c 'LD_LIBRARY_PATH=/data/local/tmp WART_APPS_ROOT=$APPS_ROOT nohup /data/local/tmp/wart-host --standalone-overlay-top --app war.statusbar >/dev/null 2>&1 &'" 2>/dev/null
            echo "▸ IME keyboard (bottom overlay) + set-ime"
            adb shell "su -c 'WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-arbiter launch-overlay war.ime.keyboard'" 2>&1 | tr -d '\r'
            sleep 1
            adb shell "su -c 'WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-arbiter set-ime war.ime.keyboard'" 2>&1 | tr -d '\r'
            break
        fi
        sleep 0.5
    done
) &

echo "▸ starting wart-arbiter --daemon in foreground (Ctrl-C to stop) …"
adb shell -t "su -c 'WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-arbiter --daemon'"
