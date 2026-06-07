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
    adb shell "su -c 'pkill -9 -f wart-framework-shim'" >/dev/null 2>&1 || true
    adb shell "su -c 'pkill -9 -f wart-activityms'" >/dev/null 2>&1 || true  # migration: clear stale old stub
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
    # Kill our framework-shim (activity/permission/...) BEFORE `start` brings the
    # real system_server back — else our shim shadows the real services.
    adb shell "su -c 'pkill -9 -f wart-framework-shim'" >/dev/null 2>&1 || true
    adb shell "su -c 'pkill -9 -f wart-activityms'" >/dev/null 2>&1 || true  # migration: clear stale old stub
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
#   --wart-only  (task 96) implies --no-art. The FAST restart path: assume the
#             native+shim layer (wart-framework-shim + sensorservice + audioserver +
#             wart-sensormanager) is ALREADY up and healthy from a prior --no-art
#             run, leave it untouched, and restart ONLY the wart layer (arbiter /
#             host zygote+hosts / inputflinger) in place. No framework stop, no
#             --restore-art boot cycle — completes in seconds. Use after the first
#             cold entry into --no-art. (Plain --no-art also skips the native+shim
#             bringup when it detects the layer already healthy — see Step 5.)
EXTRA_ENV=""
NO_ART=0
WART_ONLY=0
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
        # --wart-only (task 96): fast wart-layer-only restart on a live native+shim
        # layer. Implies --no-art (the layer only exists under --no-art).
        --wart-only) NO_ART=1; WART_ONLY=1 ;;
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
# Task 96 — the designed --no-art framework-shim (C++, built on a-03), replacing the
# ad-hoc wart-activityms stub. Registers the minimal binder service set the native
# survivors block on / query with the Java system_server stopped — derived from the
# daemon sources: waitForService blockers `activity` (audioserver UidPolicy, FATAL if
# absent), `sensor_privacy`, `package_native` (sensorservice + codec), `processinfo`;
# plus the checkService/getService fail-close/sleep-loop paths `scheduling_policy`,
# `permission`, `permission_checker`, `media.camera.proxy`. Brought up FIRST in the
# native+shim layer, before audioserver/sensorservice. See
# docs/artless-native-service-model.md, runtime/wart-framework-shim/.
SHIM_BIN="$REPO_ROOT/runtime/wart-framework-shim/cpp/wart-framework-shim"
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

# Resolve the stock home (launcher) package via the framework — used ONLY under ART
# (to force-stop SystemUI + the launcher so the runtime owns the screen). Under
# --no-art the framework is (or will be) stopped and HOME_PKG is NEVER used (the wart
# launcher is the arbiter-persisted WART_HOME_APP / war.launcher, not the stock one).
# Task 96 DROPS the framework-up gate under --no-art: skip the `cmd package` call
# entirely so a wart-only restart never depends on a live framework. (`cmd package
# resolve-activity` also returns exit 20 in some framework states, which under
# `set -o pipefail` would kill the assignment before any guard — another reason to
# skip it rather than tolerate-and-check.)
HOME_PKG=""
if [[ "$NO_ART" == "1" ]]; then
    echo "▸ --no-art: skipping stock-home resolution (not needed; wart home is arbiter-persisted)"
else
    HOME_PKG="$(
        adb shell "cmd package resolve-activity \
            -a android.intent.action.MAIN \
            -c android.intent.category.HOME" 2>/dev/null \
            | awk -F= '/packageName=/ { gsub(/[ \r]/, "", $2); print $2; exit }' || true
    )"
    if [[ -z "$HOME_PKG" ]]; then
        echo "✗ could not resolve home (launcher) package — bailing." >&2
        exit 1
    fi
    echo "▸ home package: $HOME_PKG"
fi

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
    adb shell "su -c 'pkill -9 -f wart-framework-shim'" >/dev/null 2>&1
    adb shell "su -c 'pkill -9 -f wart-activityms'" >/dev/null 2>&1  # migration: clear stale old stub
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

# Task 96 framework-shim (replaces wart-activityms). Pushed if present (built on
# a-03); started FIRST in the native+shim layer under --no-art (below). On a
# --wart-only restart the device copy is reused as-is (the layer is left running), so
# a missing local build is fine.
if [[ -f "$SHIM_BIN" ]]; then
    push_if_newer "$SHIM_BIN" "/data/local/tmp/wart-framework-shim"
    adb shell 'chmod 755 /data/local/tmp/wart-framework-shim'
elif [[ "$NO_ART" == "1" && "$WART_ONLY" != "1" ]]; then
    echo "  ⚠ wart-framework-shim missing — audio/camera/sensors will wedge under --no-art" >&2
    echo "    build on a-03: scp runtime/wart-framework-shim/{cpp/wart_framework_shim.cpp,cpp/Android.bp} a-03:~/android/lineage/external/wart-framework-shim/ && \\" >&2
    echo "    ssh a-03 'cd ~/android/lineage && source build/envsetup.sh && lunch aosp_arm64-trunk_staging-userdebug && export TARGET_RELEASE=trunk_staging && m wart-framework-shim'" >&2
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

# ── Task 96: native+shim layer (brought up ONCE, idempotently) ────────────────────
# The framework-coupled native services + the framework-shim form a layer that, once
# healthy, OUTLIVES wart-stack restarts. Restarting the wart layer (arbiter / hosts /
# inputflinger) on top of a live native+shim layer is the fast `--no-art` restart
# (no `--restore-art` boot). See docs/artless-native-service-model.md.
#
# `native_shim_healthy` — 0 (true) iff every member of the native+shim layer is up and
# serving: the framework-shim ("activity"), the sensors HAL owner + AIDL bridge
# (sensorservice gyro + frameworks.sensorservice.ISensorManager), and audio
# (media.audio_policy). Reads are PLAIN `adb shell` (no `su -c`) — `service`/`dumpsys`
# are uid-agnostic and every `su -c` under --no-art spawns a Magisk su-log spinner.
# Retry one device check up to 5×0.6s before giving up — `service`/`dumpsys`/`service
# list` can momentarily fail right after the wart layer is pkilled (binder churns while
# servicemanager processes the deaths), which is NOT the native+shim layer being down.
_health_check() {
    local label="$1" cmd="$2" pat="$3"
    for _ in 1 2 3 4 5; do
        adb shell "$cmd" 2>/dev/null | grep -qi "$pat" && return 0
        sleep 0.6
    done
    echo "  [health] $label FAIL" >&2
    return 1
}
native_shim_healthy() {
    _health_check activity           'service check activity 2>/dev/null'          'found'        || return 1
    _health_check media.audio_policy 'service check media.audio_policy 2>/dev/null' 'found'        || return 1
    # `service check <name>` (servicemanager lookup only) — NOT `service list`, which
    # PINGS every registered service and blocks on the dead wart-layer services we just
    # pkilled, so it can't finish within the retry budget → false "unhealthy".
    _health_check ISensorManager     'service check android.frameworks.sensorservice.ISensorManager/default 2>/dev/null' 'found' || return 1
    _health_check gyro               'dumpsys sensorservice 2>/dev/null'           'Gyroscope'    || return 1
    return 0
}

# `bring_up_native_shim` — stand the layer up ONCE, fresh, in the --no-art context,
# shim-FIRST so nothing wedges (task 96 steps 1+2). Assumes the Java framework is
# already stopped (system_server gone) by the caller. Idempotent-safe: each service is
# single-instance (kill-then-confirm-gone before (re)start) so we never leave two
# (the duplicate-EventQueue-spin cause).
bring_up_native_shim() {
    # ── Step 1: the framework-shim, serving BEFORE audioserver/sensorservice ──
    # audioserver's UidPolicy waitForService("activity") is FATAL if absent; sensors +
    # camera block on the others. Start exactly one (kill any stale first — a shadowed
    # dead shim churns the audioserver permission/attribution path → it restarts →
    # volumes wiped → silence). Then WAIT until it actually serves "activity".
    if adb shell 'ls /data/local/tmp/wart-framework-shim' >/dev/null 2>&1; then
        echo "▸ --no-art: framework-shim (activity/permission/sensor_privacy/package_native/… — shim-first)"
        adb shell "su -c 'pkill -9 -f wart-framework-shim; pkill -9 -f wart-activityms'" >/dev/null 2>&1 || true
        spawn_detached /data/local/tmp/wart-framework-shim.log \
            "/data/local/tmp/wart-launch /data/local/tmp/wart-framework-shim"
        # Plain `adb shell` polls (no `su -c`) — see native_shim_healthy.
        for _ in $(seq 1 20); do
            adb shell 'service check activity 2>/dev/null' 2>/dev/null | grep -q found && break
            sleep 0.5
        done
        if adb shell 'service check activity 2>/dev/null' 2>/dev/null | grep -q found; then
            echo "  framework-shim serving (activity registered)"
        else
            echo "  ⚠ framework-shim not serving 'activity' — see /data/local/tmp/wart-framework-shim.log" >&2
            adb shell "su -c 'tail -15 /data/local/tmp/wart-framework-shim.log'" 2>&1 | tr -d '\r' >&2
        fi
    else
        echo "  ⚠ wart-framework-shim missing on device — audio/camera/sensors will wedge" >&2
    fi

    # ── Step 2a: audioserver, ONCE, with the shim already serving ──
    # audioserver is `class core` (survives `stop`, init-respawns on kill) but lost its
    # system_server clients when the framework dropped → it wedges on
    # waitForService("activity") and media.audio_* unregister. With the shim now
    # serving "activity", a single `pkill` → init respawn re-registers media.audio_*
    # cleanly in ~1s (vs the ~20s wedge). One deterministic restart — not a cycle.
    echo "▸ --no-art: audioserver re-init (single, shim-first) + volume boot replica"
    adb shell "su -c 'pkill -9 audioserver'" >/dev/null 2>&1 || true
    for _ in $(seq 1 20); do
        adb shell 'service list 2>/dev/null' 2>/dev/null | grep -q "media.audio_policy:" && { echo "  audio ready (media.audio_policy registered)"; break; }
        sleep 1
    done
    # Replicate AudioService's boot volume/device init (dead with system_server):
    # without it the policy reports volume range -1 → every stream at -inf dB → silence.
    adb shell "su -c 'LD_LIBRARY_PATH=/data/local/tmp /data/local/tmp/wart-host --init-audio-policy'" >/dev/null 2>&1 || true

    # ── Step 2b: sensorservice ONCE-FRESH + wart-sensormanager ──
    # Our standalone /system/bin/sensorservice claims the single-client HIDL
    # android.hardware.sensors@1.0 HAL (the framework's sensorservice died with
    # system_server). Empirically (taimen, framework stopped): the claim succeeds on the
    # FIRST try — there is no DEAD_OBJECT churn — but the qcom SSC HAL takes ~10–15s to
    # enumerate the sensor list (measured ~13s, process alive throughout). So claim ONCE
    # and poll PATIENTLY for gyro enumeration; treat ONLY the process dying as failure,
    # because sensorservice LOG_ALWAYS_FATAL-aborts on a genuine DEAD_OBJECT
    # (HidlSensorHalWrapper.cpp:401) — an alive process WILL enumerate. A real abort
    # (rare; the dead framework client's single-slot not yet released) → wait for the HAL
    # death-recipient + re-claim, bounded. NO blind kill-retry: we never pkill a live,
    # slow-to-enumerate instance (the old 4s-snapshot bug). Plain dumpsys/pgrep (no su)
    # so the ~13s poll doesn't spawn a Magisk su-logger storm. In steady state
    # native_shim_healthy short-circuits the whole bringup, so this runs only on a cold
    # --restore-art→--no-art entry — start-once, never churn.
    if adb shell 'ls /data/local/tmp/wart-sensormanager' >/dev/null 2>&1; then
        echo "▸ --no-art: sensorservice (once-fresh HAL claim) + wart-sensormanager"
        # Kill priors by PID via device-side `pidof` (NOT `pkill -9 -f sensorservice`):
        # `pkill -f` matches the killer's OWN `su -c '…wart-sensormanager…'` cmdline and
        # races killing its own shell, so a prior wart-sensormanager can SURVIVE → two
        # instances → orphaned EventQueue CPU spin (the duplicate-instance bug). `pidof`
        # matches only the executable basename, never the killer — exactly one of each.
        adb shell "su -c 'kill -9 \$(pidof sensorservice wart-sensormanager wart-sensors) 2>/dev/null'" >/dev/null 2>&1 || true
        # Confirm the prior instances are actually gone (no competing single-client claim).
        for _ in $(seq 1 10); do
            [ -z "$(adb shell 'pidof sensorservice wart-sensormanager' | tr -d '\r')" ] && break
            sleep 0.5
        done
        ss_ok=
        for attempt in 1 2 3; do
            spawn_detached /data/local/tmp/sensorservice.log "/system/bin/sensorservice"
            seen_alive=
            for _ in $(seq 1 30); do        # up to ~30s (enumeration seen ~13s on taimen)
                sleep 1
                n=$(adb shell 'pgrep -f /system/bin/sensorservice' 2>/dev/null | tr -d '\r' | grep -c .)
                [ "$n" != "0" ] && seen_alive=1
                if [ -n "$seen_alive" ] && [ "$n" = "0" ]; then
                    echo "  sensorservice aborted (real DEAD_OBJECT) — waiting for HAL release, re-claim"
                    break
                fi
                if adb shell 'dumpsys sensorservice 2>/dev/null' 2>/dev/null | grep -qi Gyroscope; then
                    ss_ok=1; break
                fi
            done
            [ -n "$ss_ok" ] && { echo "  sensorservice owns the HAL cleanly (claim $attempt, gyro enumerated)"; break; }
            # Only here if the process died (FATAL abort) or never enumerated. Clear any
            # zombie (pidof — no self-match) + let the HAL death-recipient release the
            # slot before re-claiming.
            adb shell "su -c 'kill -9 \$(pidof sensorservice) 2>/dev/null'" >/dev/null 2>&1 || true
            sleep 3
        done
        [ -z "$ss_ok" ] && echo "  ⚠ sensorservice did not enumerate after 3 claims — see /data/local/tmp/sensorservice.log" >&2
        # wart-sensormanager (bare root; ISensorManager impls delegate to sensorservice).
        # Its ISensorManager registration lags a freshly-enumerated sensorservice while
        # the HAL settles (measured ~30s on taimen cold), so poll up to ~45s (90×0.5s)
        # before warning. This is a once-per-cold-entry cost; the --wart-only fast path
        # skips the whole bringup when native_shim_healthy.
        spawn_detached /data/local/tmp/wart-sensormanager.log "/data/local/tmp/wart-sensormanager"
        ism='service check android.frameworks.sensorservice.ISensorManager/default 2>/dev/null'
        for _ in $(seq 1 90); do
            adb shell "$ism" 2>/dev/null | grep -q found && break
            sleep 0.5
        done
        if adb shell "$ism" 2>/dev/null | grep -q found; then
            echo "  sensors ready (AIDL ISensorManager registered)"
        else
            echo "  ⚠ AIDL ISensorManager not registered — see /data/local/tmp/wart-sensormanager.log" >&2
            adb shell "su -c 'tail -15 /data/local/tmp/wart-sensormanager.log'" 2>&1 | tr -d '\r' >&2
        fi
    fi
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

# Path A (--no-art) — task 96 TWO-LAYER bringup:
#   • native+shim layer  (wart-framework-shim + sensorservice + audioserver +
#     wart-sensormanager) — brought up ONCE, idempotently; outlives wart restarts.
#   • wart layer  (wart-inputflinger here, then the zygote/arbiter below) — restartable
#     in place on top of a live native+shim layer.
# wart-inputflinger comes up BEFORE the zygote/hosts so their inputflinger client path
# (waitForService("inputflinger")) only ever resolves to OUR service, never
# system_server's (now dead). SurfaceFlinger/audioserver are native survivors, so the
# wart stack still attaches. wart-inputflinger runs via wart-launch (uid system + gid
# input + CAP_BLOCK_SUSPEND) — bare root HANGS on SF's perm check + aborts in EventHub.
if [[ "$NO_ART" == "1" ]]; then
    if [[ "$WART_ONLY" == "1" ]]; then
        # ── FAST PATH: the framework is already stopped and the native+shim layer is
        # already up from a prior --no-art run. Do NOT stop the framework and do NOT
        # touch the native+shim layer — just restart the wart layer in place. Seconds,
        # not the ~1–2 min --restore-art ART-boot cycle.
        echo "▸ --wart-only: fast wart-layer restart on the live native+shim layer (no framework boot)"
        if native_shim_healthy; then
            echo "  native+shim layer healthy (shim + sensors + audio up) — leaving it running"
        else
            echo "  ⚠ --wart-only but the native+shim layer is NOT healthy — bringing it up once" >&2
            echo "    (use a plain --no-art for a cold entry from ART)" >&2
            bring_up_native_shim
        fi
    else
        # ── COLD/NORMAL --no-art ENTRY: stop the Java framework, then bring the
        # native+shim layer up ONCE. If it is somehow already healthy (a plain --no-art
        # re-run while already in --no-art), skip the bringup — idempotent.
        echo "▸ --no-art path A: stopping Java framework (zygote + system_server) …"
        adb shell "su -c 'stop zygote; stop zygote_secondary'" >/dev/null 2>&1 || true
        # system_server is a forked child of zygote; wait for it to actually exit so its
        # "inputflinger" registration clears before ours — AND, crucially for task 96,
        # so the sensors-HAL single-client slot it held is released before our
        # standalone sensorservice claims it (the DEAD_OBJECT race; see bring_up_native_shim).
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
        # bootanimation, which draws over SurfaceFlinger and covers the wart UI.
        echo "▸ --no-art: stopping bootanimation (covers the wart UI otherwise)"
        adb shell "su -c 'setprop service.bootanim.exit 1; stop bootanim; pkill -9 -f bootanimation'" >/dev/null 2>&1 || true

        # Sweep Magisk su-loggers stuck on the boundary grants: the `magisk --sqlite`
        # disable can't suppress logging of its OWN su grant, and `stop zygote` can race
        # magiskd's policy reload — both spawn an app_process that spins ~100%/core
        # retrying the now-dead framework forever. One kill clears them.
        echo "▸ --no-art: sweeping any stuck Magisk su-loggers"
        adb shell "su -c 'pkill -9 -f com.topjohnwu.magisk'" >/dev/null 2>&1 || true

        # Native+shim layer — ONCE, skip-if-healthy. On a true cold entry the shim isn't
        # up yet (→ false → bring up); on a re-run it's already serving (→ skip).
        if native_shim_healthy; then
            echo "▸ --no-art: native+shim layer already healthy — skipping bringup (idempotent)"
        else
            bring_up_native_shim
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
