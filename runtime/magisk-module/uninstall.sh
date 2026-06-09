#!/system/bin/sh
# Runs when Magisk removes the wandr-stack module (user touched
# /data/adb/modules/wandr-stack/remove and rebooted, or used Magisk Manager
# Uninstall). Magisk has already deleted the module dir by the time this
# fires — its only job is to stop the running daemons + clean sockets so
# the system is exactly as it would have been without the module.

LOG=/data/local/tmp/wandr-stack.log

log() {
    printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >> "$LOG"
}

log "wandr-stack: uninstall.sh — stopping daemons"
killall -9 wandr-arbiter wandr-host 2>/dev/null
rm -f /data/local/tmp/wandr-zygote.sock /data/local/tmp/wandr-arbiter.sock
log "wandr-stack: uninstall complete"
