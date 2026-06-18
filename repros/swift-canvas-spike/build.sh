#!/usr/bin/env bash
# Task 114 P1 — Swift custom-WIT spike: Swift -> wasip1 reactor -> component, then
# run in a wasmtime host. Requires the wasm32-unknown-wasip1 Swift SDK.
# Verified working with Swift 6.3.2 + swift-6.3.2-RELEASE_wasm (2026-06-18).
set -euo pipefail
cd "$(dirname "$0")"
SDK="${SWIFT_WASM_SDK:-swift-6.3.2-RELEASE_wasm}"
ADAPTER=../../external/skiko/wasi_snapshot_preview1.reactor.wasm
CORE=.build/wasm32-unknown-wasip1/debug/SwiftSpike.wasm

# 1. SwiftPM cross-build to a wasip1 REACTOR module (no _start, exports `run`).
#    - reactor model via clang-linker; link the wit-bindgen component-type .o so
#      `wasm-tools component new` finds the embedded component type.
#    - SwiftPM (not raw swiftc) so the SDK's toolset wires sysroot + clang-rt.
swift build --swift-sdk "$SDK" \
  -Xswiftc -Xclang-linker -Xswiftc -mexec-model=reactor \
  -Xlinker "$PWD/generated/swift_spike_component_type.o"

# 2. wrap the wasip1 core module as a component via the preview1 adapter.
wasm-tools component new "$CORE" \
  --adapt "wasi_snapshot_preview1=$ADAPTER" \
  -o spike.component.wasm
wasm-tools validate spike.component.wasm && echo "component OK: spike.component.wasm"

# 3. run it: host provides WASI + implements wandr:swift-spike/host, calls run.
cargo run --manifest-path host/Cargo.toml --release -- spike.component.wasm
