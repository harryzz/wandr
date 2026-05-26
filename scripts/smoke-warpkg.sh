#!/usr/bin/env bash
# Build a synthetic `.warpkg` for task-35 step-6 smoke testing.
#
# Usage:
#   bash scripts/smoke-warpkg.sh [--component <path>] [--out <dir>]
#
# Defaults:
#   --component /tmp/skiko-component.wasm   (output of the CLAUDE.md build
#                                            pipeline step 3, after
#                                            wasm-tools component new --adapt)
#   --out       /tmp/smoke.warpkg
#
# Produces:
#   <out>/
#     package.toml
#     components/ui.wasm
#
# Push to the device with:
#   adb push <out> /data/local/tmp/smoke.warpkg
#
# Then on-device:
#   adb shell "su -c 'WART_APPS_ROOT=/data/local/tmp/wart-apps \
#       /data/local/tmp/wart-host --install /data/local/tmp/smoke.warpkg'"
#
# And load via:
#   adb shell "su -c 'WART_APPS_ROOT=/data/local/tmp/wart-apps \
#       LD_LIBRARY_PATH=/data/local/tmp \
#       /data/local/tmp/wart-host --standalone --app com.example.smoke'"

set -euo pipefail

COMPONENT="/tmp/skiko-component.wasm"
OUT="/tmp/smoke.warpkg"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --component) COMPONENT="$2"; shift 2 ;;
        --out)       OUT="$2";       shift 2 ;;
        -h|--help)   grep -E '^# ' "$0" | sed 's/^# \?//' ; exit 0 ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

if [[ ! -f "$COMPONENT" ]]; then
    echo "✗ component not found: $COMPONENT" >&2
    echo "  Build per CLAUDE.md 'Build pipeline' (steps 1–3) first." >&2
    exit 1
fi

rm -rf "$OUT"
mkdir -p "$OUT/components"
cp "$COMPONENT" "$OUT/components/ui.wasm"

cat > "$OUT/package.toml" <<EOF
app_id      = "com.example.smoke"
version     = "0.0.1"
world       = "my:skiko-gfx/skiko-ui"
# Task 36 / Q6: composition mode is required. Leaf consumer apps that
# never get used as a dep still must declare it. See
# tasks/36-cross-app-deps.md.
composition = "same-store"

[components]
ui = "components/ui.wasm"
EOF

echo "▸ wrote $OUT:"
ls -la "$OUT"
echo
echo "▸ size summary:"
du -sh "$OUT/components/ui.wasm" "$OUT/package.toml"
