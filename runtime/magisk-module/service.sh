#!/system/bin/sh
# wandr-stack Magisk service — boot the device straight into the wandr --no-art
# stack as the DEFAULT (task 110). Runs in Magisk's `late_start_service` stage
# as root. Stops the Java framework (system_server) and brings up the full
# --no-art stack: native+shim layer → wandr-inputflinger → zygote → arbiter →
# chrome / launcher / keyguard / power-menu.
#
# This is the on-device port of `tools/scripts/run-hybrid-stack.sh --no-art`
# (which drives the same steps over adb). KEY difference: this script already
# runs as root, so there are NO `su -c` calls — the Magisk su-logger CPU-spin
# the host script must sweep never arises here.
#
# BACKOUT — a reboot always returns to stock Android; nothing persistent is
# changed (the framework stop is runtime-only). If a bringup ever leaves no UI,
# adb still works:
#   touch /data/adb/modules/wandr-stack/disable  → next boot skips this (stock Android)
#   touch /data/adb/modules/wandr-stack/remove    → next boot removes the module
#   tools/scripts/run-hybrid-stack.sh --restore-art (or `start`) → restore now

MODDIR=${0%/*}
LOG=/data/local/tmp/wandr-stack.log
T=/data/local/tmp
APPS_ROOT=${WANDR_APPS_ROOT:-/data/local/tmp/wandr-apps}
# Set WANDR_HOME_APP="" to disable boot-to-launcher.
HOME_APP=${WANDR_HOME_APP-wandr.launcher}
HOST=$T/wandr-host
ARB=$T/wandr-arbiter
LAUNCH=$T/wandr-launch

log() { printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >>"$LOG"; }
# Detach a long-lived process from this service shell (new session, no tty).
spawn() { log "spawn: $1"; setsid sh -c "$2" </dev/null >>"${3:-$LOG}" 2>&1 & }
have() { [ -e "$1" ]; }
svc_found() { service check "$1" 2>/dev/null | grep -q found; }
# Wait up to <2nd arg>×0.5s for a unix socket; 0 = appeared.
wait_sock() { _i=0; while [ "$_i" -lt "${2:-20}" ]; do [ -S "$1" ] && return 0; sleep 0.5; _i=$((_i+1)); done; return 1; }
# Wait up to <2nd arg>×0.5s for a binder service name.
wait_svc() { _i=0; while [ "$_i" -lt "${2:-20}" ]; do svc_found "$1" && return 0; sleep 0.5; _i=$((_i+1)); done; return 1; }

log "=== wandr-stack boot (service.sh, --no-art default) ==="

[ -f "$MODDIR/disable" ] && { log "disable flag present — skipping"; exit 0; }

for b in "$HOST" "$ARB" "$LAUNCH" "$T/libsf_surface.so"; do
    have "$b" || { log "missing $b — bailing (deploy the stack first)"; exit 1; }
done

# SurfaceFlinger is a native survivor we render onto — wait for it before stopping
# the framework (early in boot it may not be up yet).
wait_svc SurfaceFlinger 60 || log "warn: SurfaceFlinger not seen (continuing)"

# ── 1. Stop the Java framework (system_server) ───────────────────────────────
log "stopping Java framework (zygote + system_server)"
stop zygote 2>/dev/null
stop zygote_secondary 2>/dev/null
_i=0; while [ "$_i" -lt 20 ] && [ -n "$(pidof system_server 2>/dev/null)" ]; do sleep 0.5; _i=$((_i+1)); done
_ssp=$(pidof system_server 2>/dev/null); [ -n "$_ssp" ] && { log "killing system_server $_ssp"; kill -9 $_ssp 2>/dev/null; sleep 1; }
# Stop bootanimation (it would draw over the wandr UI once the framework is down).
setprop service.bootanim.exit 1 2>/dev/null; stop bootanim 2>/dev/null; pkill -9 -f bootanimation 2>/dev/null

# ── 2. Native+shim layer (shim-first), only if not already healthy ───────────
native_healthy() {
    svc_found activity \
      && svc_found media.audio_policy \
      && svc_found android.frameworks.sensorservice.ISensorManager/default \
      && dumpsys sensorservice 2>/dev/null | grep -qi Gyroscope
}
if native_healthy; then
    log "native+shim layer already healthy — skipping bringup"
else
    if have "$T/wandr-framework-shim"; then
        log "framework-shim (activity/permission/sensor_privacy/…)"
        pkill -9 -f wandr-framework-shim 2>/dev/null
        pkill -9 -f wandr-activityms 2>/dev/null
        spawn framework-shim "$LAUNCH $T/wandr-framework-shim" "$T/wandr-framework-shim.log"
        wait_svc activity 20 || log "warn: framework-shim not serving 'activity'"
    fi
    # audioserver: class core (survives stop); one pkill → init respawn re-registers
    # media.audio_* cleanly now the shim serves "activity". Then replicate AudioService
    # boot volume init (else every stream is -inf dB = silence).
    log "audioserver re-init"
    pkill -9 audioserver 2>/dev/null
    _i=0; while [ "$_i" -lt 20 ]; do service list 2>/dev/null | grep -q "media.audio_policy:" && break; sleep 1; _i=$((_i+1)); done
    LD_LIBRARY_PATH=$T "$HOST" --init-audio-policy >>"$LOG" 2>&1 || true
    # sensorservice (claims the single-client HAL) + wandr-sensormanager (AIDL bridge).
    if have "$T/wandr-sensormanager"; then
        log "sensorservice + wandr-sensormanager"
        kill -9 $(pidof sensorservice wandr-sensormanager wandr-sensors 2>/dev/null) 2>/dev/null
        _i=0; while [ "$_i" -lt 10 ] && [ -n "$(pidof sensorservice wandr-sensormanager 2>/dev/null)" ]; do sleep 0.5; _i=$((_i+1)); done
        spawn sensorservice "/system/bin/sensorservice" "$T/sensorservice.log"
        _i=0; while [ "$_i" -lt 30 ]; do dumpsys sensorservice 2>/dev/null | grep -qi Gyroscope && break; sleep 1; _i=$((_i+1)); done
        spawn wandr-sensormanager "$T/wandr-sensormanager" "$T/wandr-sensormanager.log"
        wait_svc android.frameworks.sensorservice.ISensorManager/default 90 \
            || log "warn: AIDL ISensorManager not registered"
    fi
fi

# ── 3. wandr-inputflinger (uid system via wandr-launch) BEFORE the zygote ─────
if have "$T/wandr-inputflinger"; then
    log "wandr-inputflinger (path A)"
    spawn inputflinger "$LAUNCH $T/wandr-inputflinger" "$T/wandr-inputflinger.log"
    wait_svc inputflinger 20 || log "warn: inputflinger not registered"
fi

# ── 4. zygote ────────────────────────────────────────────────────────────────
log "wandr-host --zygote"
rm -f "$T/wandr-zygote.sock"
spawn zygote "LD_LIBRARY_PATH=$T WANDR_APPS_ROOT=$APPS_ROOT $HOST --zygote" "$T/wandr-zygote.log"
wait_sock "$T/wandr-zygote.sock" 30 || { log "✗ zygote socket never appeared — bailing"; exit 1; }
log "zygote up"

# ── 5. arbiter (WANDR_NO_ART=1 → arbiter owns display power) ──────────────────
log "wandr-arbiter --daemon"
rm -f "$T/wandr-arbiter.sock"
spawn arbiter "WANDR_NO_ART=1 LD_LIBRARY_PATH=$T WANDR_APPS_ROOT=$APPS_ROOT $ARB --daemon" "$T/wandr-arbiter.log"
wait_sock "$T/wandr-arbiter.sock" 30 || { log "✗ arbiter socket never appeared — bailing"; exit 1; }
log "arbiter up"

# ── 6. Connectivity + chrome / launcher / keyguard / power-menu ──────────────
if have "$T/wandr-net"; then
    log "connectivity daemon (wifi)"
    spawn wandr-net "while true; do $T/wandr-net; sleep 3; done" "$T/wandr-net.log"
fi
if [ -n "$HOME_APP" ]; then
    log "set-home $HOME_APP"
    WANDR_APPS_ROOT=$APPS_ROOT "$ARB" set-home "$HOME_APP" >>"$LOG" 2>&1 || true
fi
log "status bar + taskbar"
spawn statusbar "LD_LIBRARY_PATH=$T WANDR_APPS_ROOT=$APPS_ROOT $HOST --standalone-overlay-top --app wandr.statusbar" /dev/null
spawn taskbar "LD_LIBRARY_PATH=$T WANDR_APPS_ROOT=$APPS_ROOT $HOST --standalone-overlay-bottom-bar --app wandr.taskbar" /dev/null
log "IME keyboard + set-ime"
WANDR_APPS_ROOT=$APPS_ROOT "$ARB" launch-overlay wandr.ime.keyboard >>"$LOG" 2>&1 || true
sleep 1
WANDR_APPS_ROOT=$APPS_ROOT "$ARB" set-ime wandr.ime.keyboard >>"$LOG" 2>&1 || true
log "keyguard + boot-lock"
spawn keyguard "LD_LIBRARY_PATH=$T WANDR_APPS_ROOT=$APPS_ROOT $HOST --standalone-overlay-lock --app wandr.keyguard" /dev/null
sleep 1
WANDR_APPS_ROOT=$APPS_ROOT "$ARB" lock >>"$LOG" 2>&1 || true
log "power menu (hidden overlay)"
spawn powermenu "LD_LIBRARY_PATH=$T WANDR_APPS_ROOT=$APPS_ROOT $HOST --standalone-overlay-lock --app wandr.powermenu" /dev/null
sleep 2
WANDR_APPS_ROOT=$APPS_ROOT "$ARB" pm-dismiss >>"$LOG" 2>&1 || true

log "=== wandr-stack up (zygote + arbiter + chrome, --no-art default) ==="
