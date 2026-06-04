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
# Stop the stack: Ctrl-C in this script's terminal (interactive), or — when run
# detached / backgrounded / from CI (no TTY) — `run-hybrid-stack.sh --stop`.
#
# TTY handling: with a controlling terminal the arbiter runs in the foreground so
# Ctrl-C tears the stack down. Without one (`nohup … &`, background, CI), `adb
# shell -t` can't allocate a PTY, so the arbiter is started detached and the
# stack is left running; stop it later with `--stop`.
set -euo pipefail

# Detached stop path: tear the stack down and restore SystemUI, then exit.
if [[ "${1:-}" == "--stop" ]]; then
    echo "▸ stopping wart-arbiter + wart-host + wart-inputflinger …"
    adb shell "su -c 'pkill -9 -f wart-arbiter'" >/dev/null 2>&1 || true
    adb shell "su -c 'pkill -9 -f wart-host'"    >/dev/null 2>&1 || true
    adb shell "su -c 'pkill -9 -f wart-inputflinger'" >/dev/null 2>&1 || true
    adb shell "su -c 'rm -f /data/local/tmp/wart-zygote.sock /data/local/tmp/wart-arbiter.sock'" >/dev/null 2>&1 || true
    adb shell "su -c 'am start -n com.android.systemui/.SystemUIService'" >/dev/null 2>&1 || true
    adb shell "input keyevent KEYCODE_HOME" >/dev/null 2>&1 || true
    echo "▸ stack stopped, SystemUI restored."
    exit 0
fi

# Restore the Android Java framework after a --no-art run (task 80). adbd
# (class core, USB) survives the framework stop, so this always recovers. Path A:
# kill wart-inputflinger first so the restarting system_server re-registers the
# real platform inputflinger (else our dead service name lingers).
if [[ "${1:-}" == "--restore-art" ]]; then
    echo "▸ stopping wart stack (incl wart-inputflinger) before restoring framework …"
    adb shell "su -c 'pkill -9 -f wart-inputflinger'" >/dev/null 2>&1 || true
    adb shell "su -c 'pkill -9 -f wart-arbiter'" >/dev/null 2>&1 || true
    adb shell "su -c 'pkill -9 -f wart-host'"    >/dev/null 2>&1 || true
    echo "▸ restoring Android framework (start) …"
    adb shell "su -c 'start'" >/dev/null 2>&1 || true
    echo "▸ done — framework restarting. Re-run run-hybrid-stack.sh to bring the test stack back."
    exit 0
fi

# Flags (task 80):
#   --evdev   run hosts with WART_EVDEV_INPUT=1 (input via our standalone
#             InputReader reading /dev/input directly, not system_server).
#   --no-art  implies --evdev, and after the stack is up, stop ONLY the Java
#             framework (zygote + zygote_secondary → system_server) while keeping
#             the native survivors (surfaceflinger/audioserver/sensorservice).
#             Recover with `--restore-art`.
EXTRA_ENV=""
NO_ART=0
for arg in "$@"; do
    case "$arg" in
        # --evdev = the task-80 BOOTSTRAP path: each host runs its own evdev
        # InputReader (proves ART-less input, but global keys fan out to every host
        # → power-key flicker). Kept as a fallback / for the per-host experiment.
        --evdev)  EXTRA_ENV="WART_EVDEV_INPUT=1 " ;;
        # --no-art = PATH A: ONE wart-inputflinger service reads input + routes it
        # (focus-based for apps, system keys → arbiter once). Hosts use their normal
        # inputflinger CLIENT path (NO evdev), so EXTRA_ENV stays empty here.
        --no-art) NO_ART=1 ;;
    esac
done

# Task 81 — with ART off the arbiter owns display power (no PMS): WART_NO_ART makes
# it drive screen state from its own panel_on, force-on the panel at boot, and run
# setPowerMode as uid system via `wart-launch wart-screen` (bare root HANGS on SF's
# permission check once system_server is gone). Only the --daemon invocation needs it.
ARB_ENV=""
[[ "$NO_ART" == "1" ]] && ARB_ENV="WART_NO_ART=1 "

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

HOST_BIN="$REPO_ROOT/runtime/wart-host/target/aarch64-linux-android/release/wasm-android-host"
ARB_BIN="$REPO_ROOT/runtime/wart-arbiter/target/aarch64-linux-android/release/wart-arbiter"
SHIM="$REPO_ROOT/runtime/wart-host/cpp/build/libsf_surface.so"
# Task 81/83 — ART-off display power: wart-launch (uid-system launcher) + wart-screen
# (setPowerMode helper). Best-effort: pushed if present, only USED under --no-art.
LAUNCH_BIN="$REPO_ROOT/tools/wart-launch/wart-launch"
SCREEN_BIN="$REPO_ROOT/runtime/wart-arbiter/target/aarch64-linux-android/release/wart-screen"
# Path A (task 80) — the standalone inputflinger service (built on a-03). Only
# USED under --no-art; the platform libinputflinger.so etc. already live on-device.
INFL_BIN="$REPO_ROOT/runtime/wart-inputflinger/wart-inputflinger"
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
    adb shell "su -c 'pkill -9 -f wart-inputflinger'" >/dev/null 2>&1
    adb shell "su -c 'rm -f /data/local/tmp/wart-zygote.sock /data/local/tmp/wart-arbiter.sock'" >/dev/null 2>&1
    # Path A stops the Java framework EARLY (before the zygote), so a failure here
    # leaves it down — restart it (else `am start` below is a no-op + the device
    # has no UI owner at all). adbd survives, so this recovers.
    if [[ "${NO_ART:-0}" == "1" ]]; then
        echo "▸ --no-art failure path: restarting Android framework (start) …"
        adb shell "su -c 'start'" >/dev/null 2>&1
    fi
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

echo "▸ killing any existing wart-host / wart-arbiter / wart-inputflinger …"
adb shell "su -c 'pkill -9 -f wart-arbiter'" >/dev/null 2>&1 || true
adb shell "su -c 'pkill -9 -f wart-host'"    >/dev/null 2>&1 || true
adb shell "su -c 'pkill -9 -f wart-inputflinger'" >/dev/null 2>&1 || true
adb shell "su -c 'rm -f /data/local/tmp/wart-zygote.sock /data/local/tmp/wart-arbiter.sock'" >/dev/null 2>&1

echo "▸ pushing artifacts …"
push_if_newer "$SHIM"     "/data/local/tmp/libsf_surface.so"
push_if_newer "$HOST_BIN" "/data/local/tmp/wart-host"
push_if_newer "$ARB_BIN"  "/data/local/tmp/wart-arbiter"
adb shell 'chmod 755 /data/local/tmp/wart-host /data/local/tmp/wart-arbiter'
# Task 81/83 — display-power helpers (uid-system launcher + setPowerMode bin).
if [[ -f "$LAUNCH_BIN" && -f "$SCREEN_BIN" ]]; then
    push_if_newer "$LAUNCH_BIN" "/data/local/tmp/wart-launch"
    push_if_newer "$SCREEN_BIN" "/data/local/tmp/wart-screen"
    adb shell 'chmod 755 /data/local/tmp/wart-launch /data/local/tmp/wart-screen'
elif [[ "$NO_ART" == "1" ]]; then
    echo "✗ --no-art needs wart-launch + wart-screen (display power); build them:" >&2
    echo "    (cd tools/wart-launch && \$CC_aarch64_linux_android -O2 -o wart-launch wart-launch.c)" >&2
    echo "    (cd runtime/wart-arbiter && cargo build --release -p wart-screen)" >&2
    exit 1
fi
# Path A — the standalone inputflinger service.
if [[ -f "$INFL_BIN" ]]; then
    push_if_newer "$INFL_BIN" "/data/local/tmp/wart-inputflinger"
    adb shell 'chmod 755 /data/local/tmp/wart-inputflinger'
elif [[ "$NO_ART" == "1" ]]; then
    echo "✗ --no-art (path A) needs wart-inputflinger; build it on a-03:" >&2
    echo "    scp runtime/wart-inputflinger/{wart_inputflinger.cpp,Android.bp} a-03:~/android/lineage/external/wart-inputflinger/" >&2
    echo "    ssh a-03 'cd ~/android/lineage && source build/envsetup.sh && lunch aosp_arm64-trunk_staging-userdebug && export TARGET_RELEASE=trunk_staging && m wart-inputflinger'" >&2
    exit 1
fi

# Robustly start a long-lived device process, fully detached from this adb
# session: `setsid` (new session, no controlling tty) + stdin from /dev/null so
# the adb shell closing can't SIGHUP it. The recurring "zygote up but arbiter
# never came → stock Android reclaims the screen" wedge was a bare `nohup … &`
# that died when adb disconnected.
spawn_detached() {
    local logfile="$1" cmd="$2"
    adb shell "su -c 'setsid sh -c \"$cmd\" </dev/null >$logfile 2>&1 &'" >/dev/null 2>&1
}

# Poll up to tries×0.5 s for a device unix socket to appear. 0 = it showed up.
wait_for_sock() {
    local sock="$1" tries="${2:-40}"
    for _ in $(seq 1 "$tries"); do
        adb shell "su -c '[ -S $sock ] && echo up'" 2>/dev/null | grep -q up && return 0
        sleep 0.5
    done
    return 1
}

echo "▸ stopping SystemUI + launcher ($HOME_PKG) …"
adb shell "su -c 'am force-stop com.android.systemui'"
adb shell "su -c 'am force-stop $HOME_PKG'"

# Path A (--no-art): stop the Java framework FIRST, then start wart-inputflinger,
# BEFORE the zygote/hosts — so the hosts' inputflinger client path
# (waitForService("inputflinger")) only ever resolves to OUR service, never
# system_server's (which is now dead). SurfaceFlinger/audioserver are native
# survivors, so the wart stack still attaches. This ordering avoids any host
# input-channel "reconnect" mechanism. HOME_PKG was already resolved above (needed
# the framework). wart-inputflinger runs via wart-launch (uid system + gid input +
# CAP_BLOCK_SUSPEND) — bare root HANGS on SF's perm check + aborts in EventHub.
if [[ "$NO_ART" == "1" ]]; then
    echo "▸ --no-art path A: stopping Java framework (zygote + system_server) …"
    adb shell "su -c 'stop zygote; stop zygote_secondary'" >/dev/null 2>&1 || true
    # system_server is a forked child of zygote; give it a moment to actually exit
    # so its "inputflinger" servicemanager registration clears before ours.
    for _ in $(seq 1 20); do
        [ -z "$(adb shell "su -c 'pidof system_server'" | tr -d '\r')" ] && break
        sleep 0.5
    done
    # `|| true` INSIDE the substitution: with `set -o pipefail`, a bare
    # `x=$(… pidof …)` inherits pidof's exit 1 when the process is already gone
    # (our success case), which `set -e` would treat as fatal.
    ssp="$(adb shell "su -c 'pidof system_server'" | tr -d '\r' || true)"
    [ -n "$ssp" ] && { echo "  system_server still up ($ssp) — killing"; adb shell "su -c 'kill -9 $ssp'" >/dev/null 2>&1 || true; sleep 1; }

    # Start wart-inputflinger (registers "inputflinger") before the zygote/hosts so
    # their input client path resolves to OUR service. This gives WORKING system-key
    # dedup (POWER/VOLUME → arbiter once, no fan-out flicker). App TOUCH/key routing
    # (task 84): SurfaceFlinger can't push WindowInfos to our dispatcher under ART-off
    # (it bound the inputflinger service once at its own init / mInputFlinger, cleared
    # when system_server died, and updateInputFlinger early-returns), so instead the
    # wart-arbiter (the window manager) authors the window list and pushes it to
    # wart-inputflinger's abstract socket (@wart-inputflinger), which feeds the
    # dispatcher via onWindowInfosChanged. Hosts register their channel token with the
    # "wart.windowreg" binder service so the arbiter's per-pid windows resolve. See
    # memory project_pathA_inputflinger + tasks/84-pathA-touch-windowinfos.md.
    echo "▸ starting wart-inputflinger (path A, via wart-launch → uid system) …"
    # Forward any WART_VP_* viewport-tuning vars set in THIS script's environment so
    # the InputReader's display viewport can be aligned to SF's window coordinate
    # space on rotated panels without rebuilding the service (env inherited across
    # wart-launch's exec). Default (unset) = the service's portrait default.
    INFL_VP_ENV=""
    for v in WART_VP_LOGICAL_W WART_VP_LOGICAL_H WART_VP_DEVICE_W WART_VP_DEVICE_H WART_VP_ORIENT; do
        [ -n "${!v:-}" ] && INFL_VP_ENV="${INFL_VP_ENV}${v}=${!v} "
    done
    [ -n "$INFL_VP_ENV" ] && echo "  viewport env: $INFL_VP_ENV"
    spawn_detached /data/local/tmp/wart-inputflinger.log \
        "${INFL_VP_ENV}/data/local/tmp/wart-launch /data/local/tmp/wart-inputflinger"
    for _ in $(seq 1 20); do
        adb shell "su -c 'service check inputflinger'" 2>/dev/null | grep -q found && break
        sleep 0.5
    done
    if adb shell "su -c 'service check inputflinger'" 2>/dev/null | grep -q found; then
        echo "  inputflinger registered (wart-inputflinger) — system-key dedup active"
    else
        echo "  ⚠ inputflinger not registered — see /data/local/tmp/wart-inputflinger.log:" >&2
        adb shell "su -c 'tail -15 /data/local/tmp/wart-inputflinger.log'" 2>&1 | tr -d '\r' >&2
    fi
fi

# True-dp (Arbiter Inc. 3b): chrome heights / content insets are authored by the
# arbiter (dp×density) and reported/pushed to the hosts — no WART_INSET_* /
# WART_STATUSBAR_PX / WART_TASKBAR_PX env hardcodes. Fullscreen apps pull their
# insets via report-panel at startup; chrome overlays size their strip from the
# register-chrome reply.

echo "▸ starting wart-host --zygote (detached; insets arbiter-authored) …"
spawn_detached /data/local/tmp/wart-zygote.log \
    "${EXTRA_ENV}LD_LIBRARY_PATH=/data/local/tmp WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-host --zygote"
if ! wait_for_sock /data/local/tmp/wart-zygote.sock 30; then
    echo "✗ zygote socket never appeared — see /data/local/tmp/wart-zygote.log:" >&2
    adb shell "su -c 'tail -20 /data/local/tmp/wart-zygote.log'" 2>&1 | tr -d '\r' >&2
    exit 1
fi
ZPID="$(adb shell 'pgrep -f "wart-host --zygote" | head -1' | tr -d '\r')"
echo "  zygote up (pid $ZPID, socket present)"

# Task 57 — boot straight to the launcher: `set-home $HOME_APP` designates the
# home app AND foregrounds it. Then the status bar (top overlay), taskbar (bottom
# overlay), and IME keyboard (bottom overlay + set-ime). Each piece is
# best-effort — only fires if installed. Runs once the arbiter socket exists.
bring_up_chrome() {
    if [[ -n "$HOME_APP" ]]; then
        echo "▸ boot-to-home: set-home $HOME_APP"
        adb shell "su -c 'WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-arbiter set-home $HOME_APP'" 2>&1 | tr -d '\r'
    fi
    echo "▸ status bar (top overlay)"
    spawn_detached /dev/null "${EXTRA_ENV}LD_LIBRARY_PATH=/data/local/tmp WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-host --standalone-overlay-top --app war.statusbar"
    echo "▸ taskbar (bottom nav overlay)"
    spawn_detached /dev/null "${EXTRA_ENV}LD_LIBRARY_PATH=/data/local/tmp WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-host --standalone-overlay-bottom-bar --app war.taskbar"
    echo "▸ IME keyboard (bottom overlay) + set-ime"
    adb shell "su -c 'WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-arbiter launch-overlay war.ime.keyboard'" 2>&1 | tr -d '\r'
    sleep 1
    adb shell "su -c 'WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-arbiter set-ime war.ime.keyboard'" 2>&1 | tr -d '\r'
    echo "▸ keyguard (lock overlay) + boot-lock"
    spawn_detached /dev/null "${EXTRA_ENV}LD_LIBRARY_PATH=/data/local/tmp WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-host --standalone-overlay-lock --app war.keyguard"
    sleep 1
    # Boot = locked: the keyguard module shows the lock screen + demotes the app.
    adb shell "su -c '/data/local/tmp/wart-arbiter lock'" 2>&1 | tr -d '\r'
}

if [[ -t 1 ]]; then
    # Interactive terminal: bring chrome up in the background once the arbiter
    # socket exists, then run the arbiter in the FOREGROUND so this script blocks
    # and Ctrl-C tears the whole stack down via the EXIT/INT trap.
    ( wait_for_sock /data/local/tmp/wart-arbiter.sock 40 && bring_up_chrome ) &
    echo "▸ starting wart-arbiter --daemon in foreground (Ctrl-C to stop) …"
    adb shell -t "su -c '${ARB_ENV}WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-arbiter --daemon'"
else
    # No controlling TTY (backgrounded / nohup / CI): start the arbiter detached
    # and VERIFY its socket appears, retrying the start if it doesn't — that
    # silent miss was the "wedges after zygote, stock Android reclaims the
    # screen" bug. Then bring chrome up INLINE (no background-waiter race), and
    # exit WITHOUT firing the teardown trap so the stack keeps running.
    echo "▸ starting wart-arbiter --daemon (detached) …"
    arbiter_up=
    for attempt in 1 2 3; do
        spawn_detached /data/local/tmp/wart-arbiter.log \
            "${ARB_ENV}LD_LIBRARY_PATH=/data/local/tmp WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-arbiter --daemon"
        if wait_for_sock /data/local/tmp/wart-arbiter.sock 20; then arbiter_up=1; break; fi
        echo "  arbiter socket not up after 10 s (attempt $attempt/3) — retrying …"
        adb shell "su -c 'pkill -9 -f wart-arbiter'" >/dev/null 2>&1
    done
    if [[ -z "$arbiter_up" ]]; then
        echo "✗ wart-arbiter failed to come up after 3 attempts — see log:" >&2
        adb shell "su -c 'tail -20 /data/local/tmp/wart-arbiter.log'" 2>&1 | tr -d '\r' >&2
        exit 1   # trap restores SystemUI; a clear failure beats a silent wedge
    fi
    echo "  arbiter up (socket present)"
    bring_up_chrome
    trap - EXIT INT TERM       # don't tear the stack down on this script's exit
    if [[ "$NO_ART" == "1" ]]; then
        # Path A — the Java framework was already stopped + wart-inputflinger started
        # BEFORE the zygote (above), so the hosts connected to OUR inputflinger. The
        # native survivors (surfaceflinger/audioserver/sensorservice, class core) +
        # adbd are up; input + display power are wart-owned. Recover with --restore-art.
        echo "  --no-art path A: framework already stopped; wart-inputflinger owns input."
        echo "  Recover with: $0 --restore-art"
    fi
    echo "▸ stack up (detached). Stop with: $0 --stop"
fi
