#!/system/bin/sh
# Runs when Magisk removes the wart-stack module (user touched
# /data/adb/modules/wart-stack/remove and rebooted, or used Magisk Manager
# Uninstall). Magisk has already deleted the module dir by the time this
# fires — its only job is to stop the running daemons + clean sockets so
# the system is exactly as it would have been without the module.

LOG=/data/local/tmp/wart-stack.log

log() {
    printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >> "$LOG"
}

log "wart-stack: uninstall.sh — stopping daemons"
killall -9 wart-arbiter wart-host 2>/dev/null
rm -f /data/local/tmp/wart-zygote.sock /data/local/tmp/wart-arbiter.sock
log "wart-stack: uninstall complete"
