#!/usr/bin/env bash
# wandr.ktcanvas.test — build: Kotlin guest → component → components/ui.wasm
#
# Stage 1 of the Kotlin wasi:canvas migration. Bindings under
# src/wasmWasiMain/kotlin/bindings/ are GENERATED (./build.sh --regen) by the
# JetBrains Kotlin wit-bindgen fork — github.com/Kotlin/wit-bindgen, branch
# `kotlin`, rev pinned below — then implemented in src/.../kotlin/impl/.
# Toolchain halves stay the wandr pairing: 2.4.258-SNAPSHOT stdlib override
# (build.gradle.kts) + the wandr-fork P1 adapter (NOT the upstream reactor
# adapter the JetBrains sample ships).
set -euo pipefail
cd "$(dirname "$0")"

WANDR_ROOT="$(cd ../../.. && pwd)"
ADAPTER="$WANDR_ROOT/external/wasmtime/target/wasm32-unknown-unknown/release/wasi_snapshot_preview1.wasm"

# Pinned Kotlin wit-bindgen generator (branch `kotlin`).
WIT_BINDGEN_KOTLIN_REV=6b9cb12
WIT_BINDGEN_KOTLIN_DIR="${WIT_BINDGEN_KOTLIN_DIR:-/tmp/kotlin-wit-bindgen}"

if [[ "${1:-}" == "--regen" ]]; then
    BINDGEN="$WIT_BINDGEN_KOTLIN_DIR/target/release/wit-bindgen"
    [ -x "$BINDGEN" ] || { echo "build the generator first: git clone -b kotlin https://github.com/Kotlin/wit-bindgen $WIT_BINDGEN_KOTLIN_DIR && (cd $WIT_BINDGEN_KOTLIN_DIR && git checkout $WIT_BINDGEN_KOTLIN_REV && cargo build --release -p wit-bindgen-cli)"; exit 1; }
    echo "▸ regen bindings (Kotlin wit-bindgen $WIT_BINDGEN_KOTLIN_REV)"
    # No --generate-stubs: the implementation is hand-maintained in
    # src/wasmWasiMain/kotlin/impl/ (a stub file here would clash with it).
    rm -rf src/wasmWasiMain/kotlin/bindings
    "$BINDGEN" kotlin --kotlin-imports 'impl.*' -w ktcanvas-test wit \
        --out-dir src/wasmWasiMain/kotlin/bindings/
    exit 0
fi

[ -f "$ADAPTER" ] || { echo "missing wandr-fork adapter: $ADAPTER"; exit 1; }

echo "▸ Kotlin wasmWasi compile (2.4.258-SNAPSHOT stdlib override)"
./gradlew -q compileProductionExecutableKotlinWasmWasi

GUEST_WASM=build/compileSync/wasmWasi/main/productionExecutable/kotlin/wandr-ktcanvas-test.wasm
[ -f "$GUEST_WASM" ] || { echo "guest wasm not found: $GUEST_WASM"; exit 1; }

mkdir -p components
echo "▸ componentize (embed + new --adapt, wandr-fork P1 adapter)"
wasm-tools component embed wit "$GUEST_WASM" -w ktcanvas-test -o components/ui.embedded.wasm
wasm-tools component new components/ui.embedded.wasm \
    --adapt wasi_snapshot_preview1="$ADAPTER" \
    -o components/ui.wasm
rm components/ui.embedded.wasm

echo "OK → components/ui.wasm"
echo "desktop run: WANDR_DESKTOP_SIZE=500x1000 $WANDR_ROOT/runtime/wandr-host/target/x86_64-unknown-linux-gnu/release/wasm-android-host $(pwd)/components/ui.wasm"

# --deploy: per-app install on the device (never build-system-wandrpkgs.sh —
# it wipes APPS_ROOT). User app → no zygote restart needed; kill+launch picks
# up the new cwasm.
if [[ "${1:-}" == "--deploy" ]]; then
    APPS_ROOT=/data/local/tmp/wandr-apps
    DEV=/data/local/tmp/wandr.ktcanvas.test.wandrpkg
    HOST="LD_LIBRARY_PATH=/data/local/tmp WANDR_APPS_ROOT=$APPS_ROOT /data/local/tmp/wandr-host"
    AR="WANDR_APPS_ROOT=$APPS_ROOT /data/local/tmp/wandr-arbiter"

    PKG=build/wandr.ktcanvas.test.wandrpkg
    rm -rf "$PKG" && mkdir -p "$PKG/components"
    cp package.toml "$PKG/"
    cp components/ui.wasm "$PKG/components/"

    echo "▸ push + install (AOT precompile)"
    adb shell "rm -rf $DEV" >/dev/null 2>&1
    adb push "$PKG" "$DEV" >/dev/null
    adb shell "su -c '$HOST --install $DEV'" 2>&1 | tr -d '\r' | tail -1
    echo "▸ relaunch"
    adb shell "su -c '$AR kill wandr.ktcanvas.test'" 2>&1 | tr -d '\r' >/dev/null || true
    adb shell "su -c '$AR launch wandr.ktcanvas.test'" 2>&1 | tr -d '\r'
fi
