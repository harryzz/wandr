#!/usr/bin/env bash
# Pack + push + install the war.ime.keyboard warpkg with [dependencies]
# for the language plugins (war.lang.bg, war.lang.fr — task 49 step 5).
#
# Prereqs:
#   - war.ime.keyboard/build/.../war-ime-keyboard.wasm exists
#     (run `cd war.ime.keyboard && ./gradlew compileProductionExecutableKotlinWasmWasi`)
#   - wart-host binary on device
#   - scripts/build-system-warpkgs.sh already ran (lang plugins installed)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APPS_ROOT="${APPS_ROOT:-/data/local/tmp/wart-apps}"
WAW="${WAW:-$REPO_ROOT/apps/user/wart-app/wasi_snapshot_preview1.wasm}"
# Fall back to the wart wasmtime-src adapter (the one the wart-app
# pipeline uses — see CLAUDE.md "Build pipeline" section).
[[ -f "$WAW" ]] || WAW="$REPO_ROOT/external/wasmtime/target/wasm32-unknown-unknown/release/wasi_snapshot_preview1.wasm"

IME_WASM="$REPO_ROOT/apps/system/war.ime.keyboard/build/compileSync/wasmWasi/main/productionExecutable/kotlin/war-ime-keyboard.wasm"
if [[ ! -f "$IME_WASM" ]]; then
    echo "✗ $IME_WASM missing — run gradle compile first." >&2
    exit 1
fi
if [[ ! -f "$WAW" ]]; then
    echo "✗ wasi adapter missing — checked $WAW" >&2
    exit 1
fi

echo "▸ embed + adapt war.ime.keyboard"
wasm-tools component embed \
    --world war:ime-keyboard/ime-keyboard \
    "$REPO_ROOT/apps/system/war.ime.keyboard/wit" \
    "$IME_WASM" \
    -o /tmp/ime-keyboard-embedded.wasm
wasm-tools component new /tmp/ime-keyboard-embedded.wasm \
    --adapt "$WAW" \
    -o /tmp/ime-keyboard.wasm

echo "▸ pack ime-keyboard.warpkg"
PKG=/tmp/ime-keyboard.warpkg
rm -rf "$PKG"
mkdir -p "$PKG/components"
cp /tmp/ime-keyboard.wasm "$PKG/components/ui.wasm"
cat > "$PKG/package.toml" <<'EOF'
app_id      = "war.ime.keyboard"
version     = "0.1.0"
world       = "war:ime-keyboard/ime-keyboard"
composition = "same-store"

[components]
ui = "components/ui.wasm"

# Task 49 step 5 — language plugins. Each uses its own WIT package
# because two deps cannot share a `linker.instance(name)` entry.
# Adding a new language → install war.lang.<id>.warpkg, append an
# entry here, append matching @WasmImport in LangAdapter.kt.
[dependencies]
bg = { system = "war.lang.bg", version = "0.1.0", interface = "war:keyboard-lang-bg/lang@0.1.0" }
fr = { system = "war.lang.fr", version = "0.1.0", interface = "war:keyboard-lang-fr/lang@0.1.0" }
EOF

echo "▸ push + install"
adb shell "rm -rf /data/local/tmp/ime-keyboard.warpkg"
adb push "$PKG" "/data/local/tmp/ime-keyboard.warpkg" >/dev/null
adb shell "su -c 'rm -rf $APPS_ROOT/apps/war.ime.keyboard 2>/dev/null'"
adb shell "su -c 'LD_LIBRARY_PATH=/data/local/tmp WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-host --install /data/local/tmp/ime-keyboard.warpkg'"

echo ""
echo "▸ done. Re-launch via wart-arbiter."
