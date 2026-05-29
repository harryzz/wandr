#!/system/bin/sh
# wart-stack Magisk service script (task 46 step 5 finisher).
#
# Runs in Magisk's `late_start_service` stage — the analog of init.rc's
# `class late_start` services. By this point /data is mounted, SurfaceFlinger
# is up, the binder driver is ready, and apps are starting to launch.
#
# Behaviour:
#   1. Sanity-check the wart binaries are deployed at /data/local/tmp/.
#   2. (Optional) Patch SELinux rules via `magiskpolicy --live`. Commented
#      placeholders below — uncomment when we hit real denials. As of
#      task 46 step 4 device-verify, none were observed.
#   3. Start wart-host --zygote in the background, wait for its socket.
#   4. Start wart-arbiter --daemon in the background, wait for its socket.
#   5. Log a single "stack up" line + the pids.
#
# Logs go to /data/local/tmp/wart-stack.log so a `disable`/`remove` cycle
# leaves a forensic trail. Tail with: adb shell tail -f /data/local/tmp/wart-stack.log
#
# Reversibility (the user's "backup to restore when need" requirement):
#   - touch /data/adb/modules/wart-stack/disable → next boot skips this
#     script. Daemons don't auto-start. Nothing else on the system
#     changed; rebooting WITHOUT this module is identical to rebooting
#     BEFORE this module was installed.
#   - touch /data/adb/modules/wart-stack/remove → next boot deletes the
#     module dir entirely.
#   - magiskpolicy --live rules are scoped to the current boot session
#     and are not persisted; reboot without this module = baseline
#     SELinux state. No backup of /sepolicy needed.

MODDIR=${0%/*}
LOG=/data/local/tmp/wart-stack.log

log() {
    printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >> "$LOG"
}

log "wart-stack: service.sh starting (module $MODDIR)"

# ── 1. Sanity check ─────────────────────────────────────────────────────

WART_HOST=/data/local/tmp/wart-host
WART_ARB=/data/local/tmp/wart-arbiter
SHIM=/data/local/tmp/libsf_surface.so
APPS_ROOT=${WART_APPS_ROOT:-/data/local/tmp/wart-apps}
# Task 57 — home/launcher app foregrounded at boot. Set WART_HOME_APP=""
# to disable boot-to-launcher.
HOME_APP=${WART_HOME_APP-war.launcher}

for bin in "$WART_HOST" "$WART_ARB" "$SHIM"; do
    if [ ! -x "$bin" ] && [ ! -f "$bin" ]; then
        log "wart-stack: missing $bin — bailing. Push it via scripts/build-host-android.sh + adb push."
        exit 1
    fi
done

# ── 2. SELinux rules (commented out — task 46 step 4 needed none) ─────
#
# magiskpolicy is a live policy editor that ships with Magisk. Use it
# when wart-host or wart-arbiter starts producing avc-denied logcat
# entries. Examples (UNCOMMENT + adjust source/target context as needed):
#
#   /system/bin/magiskpolicy --live 'allow magisk init_t process { fork transition }'
#   /system/bin/magiskpolicy --live 'allow magisk shell_data_file file { open read write create }'
#
# Live rules apply only to the current boot — reboot resets baseline.
# Reapply by running this script (which is exactly what late_start_service
# does each boot).

# ── 3. Start wart-host --zygote ─────────────────────────────────────────

if pgrep -f 'wart-host --zygote' >/dev/null 2>&1; then
    log "wart-stack: wart-host --zygote already running, skipping start"
else
    rm -f /data/local/tmp/wart-zygote.sock
    # Task 56 — chrome content insets for fullscreen apps. Forked GUI
    # children inherit these; the host reserves the status-bar (top) +
    # taskbar (bottom) strips so apps don't draw under the chrome.
    INSET_TOP="${WART_STATUSBAR_PX:-132}"
    INSET_BOTTOM="${WART_TASKBAR_PX:-150}"
    log "wart-stack: starting wart-host --zygote (APPS_ROOT=$APPS_ROOT insets=$INSET_TOP/$INSET_BOTTOM)"
    nohup env LD_LIBRARY_PATH=/data/local/tmp WART_APPS_ROOT="$APPS_ROOT" \
        WART_INSET_TOP="$INSET_TOP" WART_INSET_BOTTOM="$INSET_BOTTOM" \
        "$WART_HOST" --zygote </dev/null >>"$LOG" 2>&1 &
fi

# Wait up to 10 s for the socket to appear.
i=0
while [ $i -lt 20 ] && [ ! -S /data/local/tmp/wart-zygote.sock ]; do
    sleep 0.5
    i=$((i + 1))
done
if [ ! -S /data/local/tmp/wart-zygote.sock ]; then
    log "wart-stack: ✗ zygote socket did not appear in 10 s — bailing"
    exit 1
fi
ZPID=$(pgrep -f 'wart-host --zygote' | head -1)
log "wart-stack: zygote up (pid=$ZPID)"

# ── 4. Start wart-arbiter --daemon ──────────────────────────────────────

if pgrep -f 'wart-arbiter' >/dev/null 2>&1; then
    log "wart-stack: wart-arbiter already running, skipping start"
else
    rm -f /data/local/tmp/wart-arbiter.sock
    log "wart-stack: starting wart-arbiter --daemon"
    nohup "$WART_ARB" --daemon </dev/null >>"$LOG" 2>&1 &
fi

i=0
while [ $i -lt 20 ] && [ ! -S /data/local/tmp/wart-arbiter.sock ]; do
    sleep 0.5
    i=$((i + 1))
done
if [ ! -S /data/local/tmp/wart-arbiter.sock ]; then
    log "wart-stack: ✗ arbiter socket did not appear in 10 s"
    exit 1
fi
APID=$(pgrep -f 'wart-arbiter' | head -1)
log "wart-stack: arbiter up (pid=$APID)"

# ── 5. Bring up the shell (task 55/57) ──────────────────────────────────
# Home/launcher (set-home), the top status bar (a direct top-overlay
# daemon), and the IME keyboard (bottom overlay + set-ime). All
# best-effort — only fire if installed.
if [ -n "$HOME_APP" ]; then
    log "wart-stack: set-home $HOME_APP"
    env WART_APPS_ROOT="$APPS_ROOT" "$WART_ARB" set-home "$HOME_APP" >>"$LOG" 2>&1 \
        || log "wart-stack: set-home failed (continuing)"
fi
log "wart-stack: status bar (top overlay)"
nohup env LD_LIBRARY_PATH=/data/local/tmp WART_APPS_ROOT="$APPS_ROOT" \
    "$WART_HOST" --standalone-overlay-top --app war.statusbar </dev/null >>"$LOG" 2>&1 &
log "wart-stack: IME keyboard (bottom overlay) + set-ime"
env WART_APPS_ROOT="$APPS_ROOT" "$WART_ARB" launch-overlay war.ime.keyboard >>"$LOG" 2>&1 \
    || log "wart-stack: IME launch failed (continuing)"
sleep 1
env WART_APPS_ROOT="$APPS_ROOT" "$WART_ARB" set-ime war.ime.keyboard >>"$LOG" 2>&1 || true

log "wart-stack: ✓ stack up — zygote=$ZPID arbiter=$APID home=${HOME_APP:-<none>}"
