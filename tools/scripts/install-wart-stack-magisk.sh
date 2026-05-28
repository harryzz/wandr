#!/usr/bin/env bash
# Install the wart-stack Magisk module to /data/adb/modules/wart-stack/.
# Reversible — touch the disable / remove flag on device to back out
# without an adb push. See wart-stack-magisk/README.md.
#
# Task 46 step 5 finisher. Replaces the AOSP-init.rc framing — on a
# Magisk-rooted device this is the equivalent mechanism for
# "auto-start at boot," with built-in disable/remove for backout.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MOD_SRC="$REPO_ROOT/runtime/magisk-module"
MOD_DST="/data/adb/modules/wart-stack"

if [[ ! -d "$MOD_SRC" ]]; then
    echo "✗ $MOD_SRC missing" >&2
    exit 1
fi

# Magisk requires the module dir tree to be readable by uid 0 and to
# have exec bit on the scripts. We push to a staging path the shell can
# write to, then move into place via su.
echo "▸ pushing module to /data/local/tmp/wart-stack-stage …"
adb shell 'rm -rf /data/local/tmp/wart-stack-stage'
adb push "$MOD_SRC" /data/local/tmp/wart-stack-stage >/dev/null
adb shell 'chmod 755 /data/local/tmp/wart-stack-stage/*.sh'

echo "▸ moving into $MOD_DST …"
adb shell "su -c '
    rm -rf $MOD_DST
    mv /data/local/tmp/wart-stack-stage $MOD_DST
    chown -R 0:0 $MOD_DST
    chmod 755 $MOD_DST/service.sh $MOD_DST/uninstall.sh
'"

echo "▸ on-device contents:"
adb shell "su -c 'ls -la $MOD_DST'"

echo ""
echo "✓ installed. NOT enabled until you reboot (Magisk scans"
echo "  /data/adb/modules/ at boot only)."
echo ""
echo "  enable now:  adb reboot"
echo "  disable on next boot: adb shell \"su -c 'touch $MOD_DST/disable'\""
echo "  remove on next boot:  scripts/uninstall-wart-stack-magisk.sh"
echo "  tail logs:   adb shell tail -f /data/local/tmp/wart-stack.log"
