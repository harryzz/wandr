#!/usr/bin/env bash
# Phase 4b — OpenSwiftUI renders on wandr through wasi:canvas. Builds the OpenSwiftUIDemo
# guest component: a real SwiftUI view → OpenSwiftUI DisplayList → WandrDrawSink → CGContext
# → wasi:canvas (skia/EGL). Reuses the spike's CSwiftSpike + OpenCoreGraphics + WIT; adds the
# OpenSwiftUI dep, so the build needs BOTH the spike's WASI-emulation flags AND OpenSwiftUI's
# local-deps env + clang flags. Requires /tmp/{OpenSwiftUI,oag-fork,Compute,oag-shims} (see
# repros/openswiftui-wasm/RESUME.md "Build environment").
set -euo pipefail
cd "$(dirname "$0")"
SDK="${SWIFT_WASM_SDK:-swift-6.3.2-RELEASE_wasm}"
ADAPTER=../../external/skiko/wasi_snapshot_preview1.reactor.wasm
CORE=.build/wasm32-unknown-wasip1/debug/OpenSwiftUIDemo.wasm
OUT=openswiftui-demo.component.wasm

# 0. (re)generate the wit-bindgen-c surface from wit/ and sync into the C target.
wit-bindgen c wit --out-dir generated
cp generated/swift_spike.c Sources/CSwiftSpike/swift_spike.c
cp generated/swift_spike.h Sources/CSwiftSpike/include/swift_spike.h

# 1. SwiftPM cross-build to a wasip1 REACTOR. Only compile (-Xcc) flags here; the wasm-only
#    LINK flags (-lwasi-emulated-*, -mexec-model=reactor, the component-type .o) live in
#    Package.swift scoped to .wasi, so they don't break the host builds of OpenSwiftUI's
#    Swift-macro plugin tools. Env = OpenSwiftUI local-deps + ANY_ATTRIBUTE_FIX=0.
OPENSWIFTUI_ANY_ATTRIBUTE_FIX=0 ANY_ATTRIBUTE_FIX=0 \
OPENSWIFTUI_USE_LOCAL_DEPS=1 OPENATTRIBUTEGRAPH_OPENATTRIBUTESHIMS_COMPUTE=1 \
OPENATTRIBUTEGRAPH_USE_LOCAL_DEPS=1 \
OPENRENDERBOX_LIB_SWIFT_PATH=/tmp/oag-fork/Sources/SwiftCorelibs/include \
swift build --product OpenSwiftUIDemo --swift-sdk "$SDK" \
  -Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN -Xcc -D_WASI_EMULATED_PROCESS_CLOCKS \
  -Xcc -I/tmp/oag-shims -Xcc -fno-exceptions -Xcc -DSWIFT_INLINE_NAMESPACE=__runtime

# 2. wrap as a component via the preview1 adapter.
wasm-tools component new "$CORE" \
  --adapt "wasi_snapshot_preview1=$ADAPTER" \
  -o "$OUT"
wasm-tools validate "$OUT" && echo "component OK: $OUT ($(du -h "$OUT" | cut -f1))"
wasm-tools component wit "$OUT" | grep -E "import wasi:canvas|frame-handler" | head
echo "next: run on wandr-host (desktop dev loop, task 101) for real pixels → device."
