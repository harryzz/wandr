#!/usr/bin/env bash
# Task 114 P2 — Swift drives REAL wasi:canvas. Builds the Swift guest component;
# it RENDERS on wandr-host (which implements wasi:canvas with skia) — desktop dev
# loop, then device. Requires the wasm32-unknown-wasip1 Swift SDK.
# (P1 — the custom-WIT round-trip proof — has its own self-contained runner in
#  host/; see README.)
set -euo pipefail
cd "$(dirname "$0")"
SDK="${SWIFT_WASM_SDK:-swift-6.3.2-RELEASE_wasm}"
ADAPTER=../../external/skiko/wasi_snapshot_preview1.reactor.wasm
CORE=.build/wasm32-unknown-wasip1/debug/SwiftSpike.wasm

# 0. (re)generate the wit-bindgen-c surface from wit/ and sync into the C target.
#    Keep the component-type .o in generated/ (linked below); the C source the
#    SwiftPM C target compiles lives under Sources/CSwiftSpike/.
wit-bindgen c wit --out-dir generated
cp generated/swift_spike.c Sources/CSwiftSpike/swift_spike.c
cp generated/swift_spike.h Sources/CSwiftSpike/include/swift_spike.h

# 1. SwiftPM cross-build to a wasip1 REACTOR (no _start; exports the handlers).
#    The vendored OpenCoreGraphics pulls in Foundation/CoreFoundation; on wasm that
#    needs the WASI emulation shims (signal/mman/process-clocks) — defines for the C
#    module builds + the matching link libs.
swift build --swift-sdk "$SDK" \
  -Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN -Xcc -D_WASI_EMULATED_PROCESS_CLOCKS \
  -Xlinker -lwasi-emulated-signal -Xlinker -lwasi-emulated-mman \
  -Xlinker -lwasi-emulated-process-clocks \
  -Xswiftc -Xclang-linker -Xswiftc -mexec-model=reactor \
  -Xlinker "$PWD/generated/swift_spike_component_type.o"

# 2. wrap as a component via the preview1 adapter.
wasm-tools component new "$CORE" \
  --adapt "wasi_snapshot_preview1=$ADAPTER" \
  -o spike.component.wasm
wasm-tools validate spike.component.wasm && echo "component OK: spike.component.wasm"
wasm-tools component wit spike.component.wasm | grep -E "import wasi:canvas|export render"

echo "next: render on wandr-host (P2.2) — add the frame-handler export + a"
echo "package.toml, then run on the desktop dev loop (task 101) for real pixels."
