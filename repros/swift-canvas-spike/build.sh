#!/usr/bin/env bash
# Task 114 P1 — Swift custom-WIT spike build. REQUIRES the Swift toolchain + the
# wasm32-unknown-wasi Swift SDK (not present in CI here). Starting point; expect
# iteration on the reactor link flags. See README.md.
set -euo pipefail
cd "$(dirname "$0")"
GEN=generated
ADAPTER=../../external/skiko/wasi_snapshot_preview1.reactor.wasm

# 1. Swift + generated C glue + the component-type .o -> a wasip1 reactor module.
swiftc -target wasm32-unknown-wasi \
  -I "$GEN" \
  Sources/spike.swift "$GEN/swift_spike.c" "$GEN/swift_spike_component_type.o" \
  -Xclang-linker -mexec-model=reactor \
  -o spike.core.wasm

# 2. wrap the wasip1 core module as a component via the preview1 adapter.
wasm-tools component new spike.core.wasm \
  --adapt "wasi_snapshot_preview1=$ADAPTER" \
  -o spike.component.wasm
wasm-tools validate spike.component.wasm && echo "component OK: spike.component.wasm"

# 3. run it (host implements wandr:swift-spike/host, calls run). See host/ (P1).
echo "next: a wasmtime component host implementing host.{log,draw-rect} + calling run"
