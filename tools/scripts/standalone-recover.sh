#!/usr/bin/env bash
# Backup recovery for the standalone path — restore SystemUI + the
# launcher when scripts/standalone-launch.sh's EXIT trap didn't fire
# (script killed, ssh dropped, etc).
#
# Idempotent — safe to run twice. If the display is still wedged after
# this, escalate to: adb reboot → bootloader (Power+VolDown) →
# fastboot reboot → LineageOS recovery.

set -euo pipefail

# adb on WSL pipes CRLF — strip \r before comparing.
if [[ "$(adb get-state 2>/dev/null | tr -d '\r' || true)" != "device" ]]; then
    echo "✗ no adb device" >&2
    exit 1
fi

set +e
adb shell "su -c 'pkill -9 -f wandr-host'"   >/dev/null 2>&1
adb shell "su -c 'am start -n com.android.systemui/.SystemUIService'" >/dev/null 2>&1
adb shell "input keyevent KEYCODE_HOME"     >/dev/null 2>&1
set -e

echo "✓ SystemUI + launcher restored. If display is still wedged: adb reboot."
