#!/usr/bin/env bash
# Pack + push + install the wandr.ime.keyboard wandrpkg with [dependencies]
# for the language plugins (wandr.lang.bg, wandr.lang.fr — task 49 step 5).
#
# Prereqs:
#   - wandr.ime.keyboard/build/.../wandr-ime-keyboard.wasm exists
#     (run `cd wandr.ime.keyboard && ./gradlew compileProductionExecutableKotlinWasmWasi`)
#   - wandr-host binary on device
#   - scripts/build-system-wandrpkgs.sh already ran (lang plugins installed)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APPS_ROOT="${APPS_ROOT:-/data/local/tmp/wandr-apps}"
WAW="${WAW:-$REPO_ROOT/apps/user/wandr-app/wasi_snapshot_preview1.wasm}"
# Fall back to the wandr wasmtime-src adapter (the one the wandr-app
# pipeline uses — see CLAUDE.md "Build pipeline" section).
[[ -f "$WAW" ]] || WAW="$REPO_ROOT/external/wasmtime/target/wasm32-unknown-unknown/release/wasi_snapshot_preview1.wasm"

IME_WASM="$REPO_ROOT/apps/system/wandr.ime.keyboard/build/compileSync/wasmWasi/main/productionExecutable/kotlin/wandr-ime-keyboard.wasm"
if [[ ! -f "$IME_WASM" ]]; then
    echo "✗ $IME_WASM missing — run gradle compile first." >&2
    exit 1
fi
if [[ ! -f "$WAW" ]]; then
    echo "✗ wasi adapter missing — checked $WAW" >&2
    exit 1
fi

echo "▸ embed + adapt wandr.ime.keyboard"
wasm-tools component embed \
    --world wandr:ime-keyboard/ime-keyboard \
    "$REPO_ROOT/apps/system/wandr.ime.keyboard/wit" \
    "$IME_WASM" \
    -o /tmp/ime-keyboard-embedded.wasm
wasm-tools component new /tmp/ime-keyboard-embedded.wasm \
    --adapt "$WAW" \
    -o /tmp/ime-keyboard.wasm

echo "▸ pack ime-keyboard.wandrpkg"
PKG=/tmp/ime-keyboard.wandrpkg
rm -rf "$PKG"
mkdir -p "$PKG/components"
cp /tmp/ime-keyboard.wasm "$PKG/components/ui.wasm"
# Manifest is the app's own apps/system/wandr.ime.keyboard/package.toml.
cp "$REPO_ROOT/apps/system/wandr.ime.keyboard/package.toml" "$PKG/package.toml"

echo "▸ push + install"
adb shell "rm -rf /data/local/tmp/ime-keyboard.wandrpkg"
adb push "$PKG" "/data/local/tmp/ime-keyboard.wandrpkg" >/dev/null
adb shell "su -c 'rm -rf $APPS_ROOT/apps/wandr.ime.keyboard 2>/dev/null'"
adb shell "su -c 'LD_LIBRARY_PATH=/data/local/tmp WANDR_APPS_ROOT=$APPS_ROOT /data/local/tmp/wandr-host --install /data/local/tmp/ime-keyboard.wandrpkg'"

echo ""
echo "▸ done. Re-launch via wandr-arbiter."
