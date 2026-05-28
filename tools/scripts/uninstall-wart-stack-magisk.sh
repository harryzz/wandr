#!/usr/bin/env bash
# Stop the wart-stack daemons immediately + queue the Magisk module for
# removal at next boot. Idempotent.
set -euo pipefail

MOD_DST="/data/adb/modules/wart-stack"

echo "▸ stopping any running daemons …"
adb shell "su -c 'killall -9 wart-arbiter wart-host 2>/dev/null'" || true
adb shell "su -c 'rm -f /data/local/tmp/wart-zygote.sock /data/local/tmp/wart-arbiter.sock'"

echo "▸ flagging module for next-boot removal …"
adb shell "su -c '
    if [ -d $MOD_DST ]; then
        touch $MOD_DST/remove
        echo \"  $MOD_DST/remove created — Magisk will delete the module on next boot\"
    else
        echo \"  $MOD_DST not present (already removed?)\"
    fi
'"

echo ""
echo "✓ daemons stopped + module flagged for removal."
echo "  Reboot to complete the removal: adb reboot"
echo "  Or undo and keep the module: adb shell \"su -c 'rm $MOD_DST/remove'\""
