#!/usr/bin/env bash
# Build + run the export-record spike: Kotlin guest → component → Rust runner.
set -euo pipefail
cd "$(dirname "$0")"

WANDR_ROOT="$(cd ../.. && pwd)"
ADAPTER="$WANDR_ROOT/external/wasmtime/target/wasm32-unknown-unknown/release/wasi_snapshot_preview1.wasm"
[ -f "$ADAPTER" ] || { echo "missing wandr-fork adapter: $ADAPTER"; exit 1; }

echo "▸ guest (Kotlin wasmWasi, 2.4.258-SNAPSHOT stdlib override)"
(cd guest && ./gradlew -q compileProductionExecutableKotlinWasmWasi)

GUEST_WASM=guest/build/compileSync/wasmWasi/main/productionExecutable/kotlin/kt-export-record-spike.wasm
[ -f "$GUEST_WASM" ] || { echo "guest wasm not found: $GUEST_WASM"; exit 1; }

mkdir -p build
echo "▸ componentize (embed + new --adapt, wandr-fork P1 adapter)"
wasm-tools component embed wit "$GUEST_WASM" -w spike-guest -o build/embedded.wasm
wasm-tools component new build/embedded.wasm \
    --adapt wasi_snapshot_preview1="$ADAPTER" \
    -o build/spike.component.wasm

echo "▸ runner (wasmtime 45 desktop JIT)"
(cd runner && cargo run --release --quiet -- ../build/spike.component.wasm "${1:-100000}")
