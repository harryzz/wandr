#!/usr/bin/env bash
# [wandr TEMPORARY] Build T2iles as a HEADLESS command (-DWANDR_HEADLESS) that deterministically
# reproduces the AttributeGraph teardown UAF with no host/canvas. Run the output under wasmtime.
set -euo pipefail
cd "$(dirname "$0")"
SDK="${SWIFT_WASM_SDK:-swift-6.3.2-RELEASE_wasm}"

OPENSWIFTUI_ANY_ATTRIBUTE_FIX=0 ANY_ATTRIBUTE_FIX=0 \
OPENSWIFTUI_USE_LOCAL_DEPS=1 OPENATTRIBUTEGRAPH_OPENATTRIBUTESHIMS_COMPUTE=1 \
OPENATTRIBUTEGRAPH_USE_LOCAL_DEPS=1 \
OPENSWIFTUI_SWIFT_CRYPTO=0 \
OPENRENDERBOX_LIB_SWIFT_PATH=/home/harry/wandr/swift/OpenSwiftUIProject/OpenAttributeGraph/Sources/SwiftCorelibs/include \
swift build --product T2iles --swift-sdk "$SDK" --manifest-cache none \
  -Xswiftc -DWANDR_HEADLESS \
  -Xswiftc -enforce-exclusivity=unchecked \
  -Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN -Xcc -D_WASI_EMULATED_PROCESS_CLOCKS \
  -Xcc -I/home/harry/wandr/swift/OpenSwiftUIProject/wandr/wasi-shims \
  -Xcc -include -Xcc /home/harry/wandr/swift/OpenSwiftUIProject/wandr/wasi-shims/wasi_compat.h \
  -Xcc -fno-exceptions -Xcc -DSWIFT_INLINE_NAMESPACE=__runtime \
  -Xcc -ffunction-sections -Xcc -fdata-sections -Xlinker --gc-sections \
  -Xlinker -z -Xlinker stack-size=8388608
echo "BUILD_EXIT=$?"
echo "CORE=.build/wasm32-unknown-wasip1/debug/T2iles.wasm"
