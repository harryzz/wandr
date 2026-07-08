#!/usr/bin/env bash
# Task 115 M2a spike — build guests, compose (gate: wac accepts the async edge), build host.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"

(cd "$HERE/engine" && cargo build --release --target wasm32-wasip2)
(cd "$HERE/ui" && cargo build --release --target wasm32-wasip2)
(cd "$HERE/p2sync" && cargo build --release --target wasm32-wasip2)

# Gate: wac must accept the async-lifted export both plugged into the ui's
# sync-lowered import AND re-exported on the composite (host calls `start`).
wac compose \
  -d demo:engine="$HERE/engine/target/wasm32-wasip2/release/cma_engine.wasm" \
  -d demo:ui="$HERE/ui/target/wasm32-wasip2/release/cma_ui.wasm" \
  -o "$HERE/composite.wasm" \
  "$HERE/compose.wac"
# cm-async isn't in the validator's default feature set yet (wasm-tools 1.245).
wasm-tools validate -f cm-async "$HERE/composite.wasm"
echo "composite OK: $HERE/composite.wasm"

cargo build --release --manifest-path "$HERE/host/Cargo.toml"
echo
echo "run: $HERE/host/target/release/cma-cross-call-spike-host $HERE/composite.wasm $HERE/p2sync/target/wasm32-wasip2/release/cma_p2sync.wasm"
