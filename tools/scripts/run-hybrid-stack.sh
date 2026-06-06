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
    adb shell "su -c 'pkill -9 -f wart-sensormanager'" >/dev/null 2>&1 || true
    adb shell "su -c 'pkill -9 -f /system/bin/sensorservice'" >/dev/null 2>&1 || true
    adb shell "su -c 'pkill -9 -f wart-net'" >/dev/null 2>&1 || true
    adb shell "su -c 'pkill -9 -f wart-activityms'" >/dev/null 2>&1 || true
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
    # Kill OUR standalone sensorservice + wart-sensormanager so the restored
    # system_server can re-own the (single-client) sensors HAL with its own
    # in-process SensorService (task 94).
    adb shell "su -c 'pkill -9 -f wart-sensormanager'" >/dev/null 2>&1 || true
    adb shell "su -c 'pkill -9 -f /system/bin/sensorservice'" >/dev/null 2>&1 || true
    adb shell "su -c 'pkill -9 -f wart-net'" >/dev/null 2>&1 || true
    # Kill our stub system_server (activity/permission/...) BEFORE `start` brings the
    # real system_server back — else our stub shadows the real services.
    adb shell "su -c 'pkill -9 -f wart-activityms'" >/dev/null 2>&1 || true
    adb shell "su -c 'pkill -9 -f wart-arbiter'" >/dev/null 2>&1 || true
    adb shell "su -c 'pkill -9 -f wart-host'"    >/dev/null 2>&1 || true
    # Also kill any stuck Magisk su-loggers spinning against the (dead) framework,
    # then restore Magisk su logging/notification (disabled on --no-art entry).
    adb shell "su -c 'pkill -9 -f \"com.topjohnwu.magisk\"'" >/dev/null 2>&1 || true
    adb shell "su -c 'command -v magisk >/dev/null 2>&1 && magisk --sqlite \"UPDATE policies SET logging=1, notification=1\"'" >/dev/null 2>&1 || true
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
# Task 93/94 — ART-off sensors: wart-sensormanager (C++, built on a-03) registers the
# frameworks ISensorManager (HIDL for the camera EIS + AIDL for the wart Rust stack)
# on top of the native /system/bin/sensorservice (the HAL owner). The arbiter's
# task-77 sensor-driver then consumes the AIDL endpoint for auto-rotation / proximity
# / auto-brightness — the old `wart-sensors` direct-HAL daemon is retired.
SENSORMGR_BIN="$REPO_ROOT/runtime/wart-sensormanager/cpp/wart-sensormanager"
# Task 88 — ART-off connectivity: the wart-net daemon (Rust) brings WiFi-STA up
# (spawn vendor wpa_supplicant + ctrl-socket associate → pure-Rust DHCPv4 →
# apply address) as root, then drives netd + dnsresolver over binder AS UID
# SYSTEM (INetd networkCreate/AddRoute/SetDefault + IDnsResolver
# setResolverConfiguration — both reject root) for routing + DNS, and reports
# link state to the arbiter. Only launched under --no-art.
# NOTE: assumes the WiFi chip is already powered + STA iface up (WiFi was ON
# under ART before --no-art); cold chip power-up via the IWifi HAL is M2.
WART_NET_BIN="$REPO_ROOT/runtime/wart-net/target/aarch64-linux-android/release/wart-net"
# ART-off audio: a stub "activity" (IActivityManager) + "sensor_privacy" binder
# service (C++, built on a-03). audioserver/cameraserver block on
# waitForService("activity")/("sensor_privacy") (those live in the dead system_server)
# → audioserver wedges in init → media.audio_* never register → no audio. The stub
# unblocks them. See [[project-artless-audio]].
ACTIVITYMS_BIN="$REPO_ROOT/runtime/wart-activityms/cpp/wart-activityms"
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
    adb shell "su -c 'pkill -9 -f wart-sensormanager'" >/dev/null 2>&1
    adb shell "su -c 'pkill -9 -f /system/bin/sensorservice'" >/dev/null 2>&1
    adb shell "su -c 'pkill -9 -f wart-net'" >/dev/null 2>&1
    adb shell "su -c 'pkill -9 -f wart-activityms'" >/dev/null 2>&1
    adb shell "su -c 'rm -f /data/local/tmp/wart-zygote.sock /data/local/tmp/wart-arbiter.sock'" >/dev/null 2>&1
    # Path A stops the Java framework EARLY (before the zygote), so a failure here
    # leaves it down — restart it (else `am start` below is a no-op + the device
    # has no UI owner at all). adbd survives, so this recovers.
    if [[ "${NO_ART:-0}" == "1" ]]; then
        echo "▸ --no-art failure path: restarting Android framework (start) …"
        adb shell "su -c 'pkill -9 -f \"com.topjohnwu.magisk\"'" >/dev/null 2>&1
        magisk_su_logging 1  # restore Magisk su logging (disabled on --no-art entry)
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
adb shell "su -c 'pkill -9 -f wart-net'" >/dev/null 2>&1 || true
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
# Task 93/94 — ART-off sensors: the wart-sensormanager service (built on a-03).
# Best-effort: pushed if present; only launched under --no-art. /system/bin/
# sensorservice (the HAL owner it sits on top of) is already on-device.
if [[ -f "$SENSORMGR_BIN" ]]; then
    push_if_newer "$SENSORMGR_BIN" "/data/local/tmp/wart-sensormanager"
    adb shell 'chmod 755 /data/local/tmp/wart-sensormanager'
elif [[ "$NO_ART" == "1" ]]; then
    echo "  ⚠ wart-sensormanager missing — no sensors (auto-rotation/proximity/brightness) under --no-art" >&2
    echo "    build on a-03: m wart-sensormanager (see runtime/wart-sensormanager/cpp/Android.bp)" >&2
fi
# Task 88 — ART-off connectivity daemon. Best-effort: pushed if present; only
# launched under --no-art (below).
if [[ -f "$WART_NET_BIN" ]]; then
    push_if_newer "$WART_NET_BIN" "/data/local/tmp/wart-net"
    adb shell 'chmod 755 /data/local/tmp/wart-net'
elif [[ "$NO_ART" == "1" ]]; then
    echo "  ⚠ wart-net missing — no WiFi under --no-art" >&2
    echo "    build: (cd runtime/wart-net && cargo build --release)" >&2
fi

# ART-off audio stub (activity + sensor_privacy). Pushed if present (built on a-03);
# launched under --no-art (below).
if [[ -f "$ACTIVITYMS_BIN" ]]; then
    push_if_newer "$ACTIVITYMS_BIN" "/data/local/tmp/wart-activityms"
    adb shell 'chmod 755 /data/local/tmp/wart-activityms'
elif [[ "$NO_ART" == "1" ]]; then
    echo "  ⚠ wart-activityms missing — audio (audioserver) will wedge under --no-art" >&2
    echo "    build on a-03: ninja -f out/combined-aosp_arm64.ninja out/soong/.intermediates/external/wart-activityms/..." >&2
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

# Magisk su-logging control (high-CPU fix under --no-art). On every `su -c`, Magisk
# logs the grant + notifies its manager app via `am`/`content` against the
# framework. With ART stopped the framework is dead, so each of those spawns an
# app_process that spins ~100% on a core retrying ActivityManager forever — and the
# stack + scripts issue many `su -c`. So under --no-art we set the policies'
# logging/notification to 0 BEFORE stopping the framework (no backlog is ever
# queued), and restore them to 1 on --restore-art. No-op if Magisk isn't installed.
magisk_su_logging() {
    local on="$1" # 1 = enable (normal), 0 = disable (quiet)
    adb shell "su -c 'command -v magisk >/dev/null 2>&1 && \
        magisk --sqlite \"UPDATE policies SET logging=$on, notification=$on\"'" \
        >/dev/null 2>&1 || true
}

# --no-art high-CPU fix — the part magisk_su_logging CANNOT cover. The magiskd that
# started at boot keeps its policy CACHED, so even with logging=0 in the DB every
# `su -c` of bringup still makes magiskd fork a worker that runs `am ... action log`
# to notify the (now dead) framework. `am` can never reach ActivityManager, so the
# worker loops it forever (new PID each retry, ~100% of a core) and they ACCUMULATE
# across the many `su -c` of bringup → pinned cores + a hot phone (measured: ~260%
# busy vs ~14% once cleared). The am logger's PARENT is the stuck worker: kill the
# parents (stops the respawn) + the am children. The MAIN magiskd has no am child,
# so it survives and `su` keeps working. MUST run AFTER all bringup `su -c`; a 2nd
# pass (same su session, no new grant) catches the worker this sweep itself spawns.
magisk_worker_sweep() {
    adb shell 'cat > /data/local/tmp/wart-magisk-sweep.sh' <<'SWEEP' 2>/dev/null || true
#!/system/bin/sh
kw() {
  for am in $(pgrep -f com.android.commands.am.Am 2>/dev/null); do
    p=$(awk '{print $4}' "/proc/$am/stat" 2>/dev/null)
    [ -n "$p" ] && [ "$p" != "1" ] && kill -9 "$p" 2>/dev/null
  done
  pkill -9 -f com.android.commands.am.Am 2>/dev/null
}
kw; sleep 4; kw
SWEEP
    adb shell "su -c 'sh /data/local/tmp/wart-magisk-sweep.sh'" >/dev/null 2>&1 || true
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

# --no-art high-CPU fix: every `su -c` makes Magisk log/notify the grant via
# `am`/`content` against the framework; with ART stopped those spawn an app_process
# that spins ~100%/core retrying ActivityManager forever, and the bringup issues
# many. Two parts: (1) quiet Magisk su-logging up front; (2) SKIP the SystemUI/
# launcher force-stops — `stop zygote` kills them anyway, and they're the one su that
# runs at the framework-up/down boundary, where a logger they spawn can still be
# in-flight when the framework drops and gets stuck. Restored on --restore-art.
if [[ "$NO_ART" == "1" ]]; then
    echo "▸ --no-art: disabling Magisk su logging/notification (avoids ~100%/core spin)"
    magisk_su_logging 0
else
    echo "▸ stopping SystemUI + launcher ($HOME_PKG) …"
    adb shell "su -c 'am force-stop com.android.systemui'"
    adb shell "su -c 'am force-stop $HOME_PKG'"
fi

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

    # Stop the boot animation: with the framework down init can (re)start
    # bootanimation, which draws over SurfaceFlinger and covers the wart UI. Ask it
    # to exit (the canonical setprop), stop the init service, and hard-kill as a
    # backstop.
    echo "▸ --no-art: stopping bootanimation (covers the wart UI otherwise)"
    adb shell "su -c 'setprop service.bootanim.exit 1; stop bootanim; pkill -9 -f bootanimation'" >/dev/null 2>&1 || true

    # Sweep Magisk su-loggers stuck on the boundary grants: the `magisk --sqlite`
    # disable can't suppress logging of its OWN su grant, and `stop zygote` can race
    # magiskd's policy reload — both spawn an app_process that spins ~100%/core
    # retrying the now-dead framework forever. They never respawn, so one kill clears
    # them; `logging=0` is in effect now, so this sweep isn't itself logged. (The bulk
    # of bringup su is already quiet from the disable above.)
    echo "▸ --no-art: sweeping any stuck Magisk su-loggers"
    adb shell "su -c 'pkill -9 -f com.topjohnwu.magisk'" >/dev/null 2>&1 || true

    # ART-off audio: bring up the stub activity/sensor_privacy service + re-init
    # audioserver NOW — BEFORE the zygote/apps — and WAIT for the audio services to
    # register before continuing. audioserver wedges on waitForService("activity")
    # when the framework drops (→ media.audio_* unregister); the stub unblocks it.
    # Doing it here (not after apps launch) means audio is ready before any app opens
    # an audio stream — avoiding the ~20s openStream block an app hits when it races a
    # late audioserver restart, and letting the HAL settle before streams open. See
    # [[project-artless-audio]].
    if adb shell 'ls /data/local/tmp/wart-activityms' >/dev/null 2>&1; then
        echo "▸ --no-art: audio stub (activity + sensor_privacy) + audioserver re-init"
        # Kill any prior stub host FIRST — each bringup re-addService's the same
        # names (activity/permission/sensor_privacy/scheduling_policy), and leaking
        # old hosts churns those registrations: the audioserver caches binder handles
        # to them, so a shadowed/dead stub makes the permission/attribution path
        # (output-stream open, and `dumpsys media.audio_flinger`) wedge → audioserver
        # restarts → stream volumes wiped → call goes silent. Exactly one must run.
        adb shell "su -c 'pkill -9 -f wart-activityms'" >/dev/null 2>&1 || true
        spawn_detached /data/local/tmp/wart-activityms.log \
            "/data/local/tmp/wart-launch /data/local/tmp/wart-activityms"
        # The poll loops use PLAIN `adb shell` (NOT `su -c`): `service list` is a
        # read any uid can do, and crucially every `su -c` under --no-art spawns a
        # crashing Magisk su-log `am` process — 40 polls × su = a logger storm that
        # pins the CPU and slows the very audioserver/openStream we're waiting on.
        #
        # Wait for the stub to register "activity" BEFORE kicking audioserver: if
        # activity is up when audioserver restarts it re-registers the audio services
        # in ~1s; if it restarts wedged (no activity) it stalls ~20s (and apps then
        # block that long in openStream).
        for _ in $(seq 1 20); do
            adb shell 'service list 2>/dev/null' 2>/dev/null | grep -q "activity:" && break
            sleep 0.5
        done
        adb shell "su -c 'pkill -9 audioserver'" >/dev/null 2>&1 || true
        # Wait for the (non-lazy) audio policy service to re-register — fast now that
        # activity is up — so apps never block in openStream. (media.aaudio is lazy:
        # it only appears once a client opens a stream, so don't poll for it here.)
        for _ in $(seq 1 20); do
            if adb shell 'service list 2>/dev/null' 2>/dev/null | grep -q "media.audio_policy:"; then
                echo "  audio ready (media.audio_policy registered)"
                break
            fi
            sleep 1
        done
        # Replicate AudioService's boot volume/device init (dead with system_server):
        # initStreamVolume + per-device index + NORMAL mode. Without it the policy
        # reports volume range -1 and every stream sits at -inf dB → silence even
        # though streams open + route. Native Rust stand-in; see audio_policy_impl.rs.
        echo "  --no-art: initializing audio volumes (AudioService boot replica)"
        adb shell "su -c 'LD_LIBRARY_PATH=/data/local/tmp /data/local/tmp/wart-host --init-audio-policy'" >/dev/null 2>&1 || true
    fi

    # Task 93/94 — ART-off sensors: stand up the native /system/bin/sensorservice
    # (owns the single-client sensors HAL + registers the "sensorservice" binder) and
    # wart-sensormanager (publishes the frameworks ISensorManager — HIDL for the camera
    # EIS, AIDL for the wart Rust stack). Done HERE, BEFORE the zygote/arbiter, so the
    # arbiter's sensor-driver resolves the AIDL endpoint at spawn() (it caches the
    # successful lookup). Depends on wart-activityms's package_native/activity stubs
    # (started just above). This REPLACES the old wart-sensors direct-HAL daemon.
    if adb shell 'ls /data/local/tmp/wart-sensormanager' >/dev/null 2>&1; then
        echo "▸ --no-art: sensorservice + wart-sensormanager (rotation/proximity/brightness + camera EIS)"
        # Start each as BARE ROOT (task-93's proven recipe). The single-client ISensors
        # HAL must be claimed cleanly: uid-system (wart-launch) does NOT cleanly own it,
        # and a stale/competing client → "Abort due to ISensors … DEAD_OBJECT". The
        # framework was stopped above, so the HAL is free — claim it ONCE, verify clean
        # (gyro enumerates + no abort), retry if not. (spawn_detached runs su -c = root.)
        adb shell "su -c 'pkill -9 -f /system/bin/sensorservice; pkill -9 -f wart-sensormanager; pkill -9 -f wart-sensors'" >/dev/null 2>&1 || true
        sleep 2
        for attempt in 1 2 3; do
            adb shell "su -c 'pkill -9 -f /system/bin/sensorservice'" >/dev/null 2>&1 || true; sleep 1
            spawn_detached /data/local/tmp/sensorservice.log "/system/bin/sensorservice"
            sleep 4
            if adb shell "su -c 'dumpsys sensorservice 2>/dev/null'" 2>/dev/null | grep -qi Gyroscope \
               && ! adb shell "su -c 'grep -i \"Abort due to ISensors\" /data/local/tmp/sensorservice.log'" 2>/dev/null | grep -qi Abort; then
                echo "  sensorservice owns the HAL cleanly (attempt $attempt)"; break
            fi
            echo "  sensorservice HAL-claim attempt $attempt unclean (DEAD_OBJECT) — retry"
        done
        # Then wart-sensormanager (bare root; ISensorManager impls delegate to sensorservice).
        spawn_detached /data/local/tmp/wart-sensormanager.log \
            "/data/local/tmp/wart-sensormanager"
        for _ in $(seq 1 20); do
            adb shell 'service list 2>/dev/null' 2>/dev/null | grep -q 'frameworks.sensorservice.ISensorManager' && break
            sleep 0.5
        done
        if adb shell 'service list 2>/dev/null' 2>/dev/null | grep -q 'frameworks.sensorservice.ISensorManager'; then
            echo "  sensors ready (AIDL ISensorManager registered)"
        else
            echo "  ⚠ AIDL ISensorManager not registered — see /data/local/tmp/wart-sensormanager.log" >&2
            adb shell "su -c 'tail -15 /data/local/tmp/wart-sensormanager.log'" 2>&1 | tr -d '\r' >&2
        fi
    fi

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
    # Task 94 — sensors now flow through the arbiter's own task-77 sensor-driver
    # (wart-hal-sensors → the AIDL ISensorManager registered by wart-sensormanager,
    # stood up before the zygote above). No separate sensor daemon: auto-rotation,
    # proximity-screen-off, and auto-brightness are arbiter modules. (The old
    # wart-sensors direct-HAL daemon was deleted.)
    # Task 88 — under --no-art, start the connectivity daemon (WiFi). WifiService /
    # ConnectivityService die with ART; wart-net drives the native survivors
    # (wpa_supplicant + DHCPv4 + ip route + DNS) and reports to the arbiter
    # (report-net-state). Runs as root via spawn_detached's `su -c` (nl80211 /
    # route / port 68 need it). Respawn-supervised like wart-sensors: on a link
    # drop or bring-up failure the daemon exits non-zero and the loop retries.
    if [[ "$NO_ART" == "1" ]] && adb shell 'ls /data/local/tmp/wart-net' >/dev/null 2>&1; then
        echo "▸ connectivity daemon (WiFi, --no-art)"
        spawn_detached /data/local/tmp/wart-net.log \
            "while true; do /data/local/tmp/wart-net; sleep 3; done"
    fi
    # (ART-off audio stub + audioserver re-init now runs EARLY, before the zygote —
    # see the framework-stop section — so audio is ready before any app launches.)
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

    # --no-art high-CPU fix: clear the Magisk su-log workers accumulated across ALL
    # the `su -c` of bringup (the mid-bringup sweep above runs before zygote/chrome,
    # so every `spawn_detached` + arbiter call after it leaves a fresh stuck worker).
    # This runs last, after bringup's final `su -c`. --no-art only (under ART `am`
    # works, so there's nothing stuck and a legit `am`'s parent must not be killed).
    if [[ "$NO_ART" == "1" ]]; then
        echo "▸ --no-art: clearing stuck Magisk su-log workers (high-CPU fix)"
        magisk_worker_sweep
    fi
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
