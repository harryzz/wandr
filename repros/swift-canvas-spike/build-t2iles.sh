#!/usr/bin/env bash
# Build the REAL eleev/swiftui-2048 (target T2iles, unmodified game behind the shims) to a
# wasip1 reactor component and deploy as ui.wasm to the Linux + Windows dev hosts. Mirrors
# build-openswiftui-demo.sh's OpenSwiftUI local-deps env, but for --product T2iles.
set -euo pipefail
cd "$(dirname "$0")"
SDK="${SWIFT_WASM_SDK:-swift-6.3.2-RELEASE_wasm}"
ADAPTER=../../external/skiko/wasi_snapshot_preview1.reactor.wasm
CORE=.build/wasm32-unknown-wasip1/debug/T2iles.wasm
OUT=t2iles.component.wasm
LINUX_UI=/home/harry/wandr-desktop-apps/apps/wandr.swiftui.demo/0.1.0/components/ui.wasm
WIN_UI=/mnt/c/wandr-win/apps/apps/wandr.swiftui.demo/0.1.0/components/ui.wasm

# 0. regenerate the wit-bindgen-c surface.
wit-bindgen c wit --out-dir generated
cp generated/swift_spike.c Sources/CSwiftSpike/swift_spike.c
cp generated/swift_spike.h Sources/CSwiftSpike/include/swift_spike.h

# 1. SwiftPM cross-build to a wasip1 REACTOR (--product T2iles).
OPENSWIFTUI_ANY_ATTRIBUTE_FIX=0 ANY_ATTRIBUTE_FIX=0 \
OPENSWIFTUI_USE_LOCAL_DEPS=1 OPENATTRIBUTEGRAPH_OPENATTRIBUTESHIMS_COMPUTE=1 \
OPENATTRIBUTEGRAPH_USE_LOCAL_DEPS=1 \
OPENSWIFTUI_SWIFT_CRYPTO=0 \
OPENRENDERBOX_LIB_SWIFT_PATH=/home/harry/wandr/swift/OpenSwiftUIProject/OpenAttributeGraph/Sources/SwiftCorelibs/include \
swift build --product T2iles --swift-sdk "$SDK" --manifest-cache none \
  -Xswiftc -enforce-exclusivity=unchecked \
  -Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN -Xcc -D_WASI_EMULATED_PROCESS_CLOCKS \
  -Xcc -I/home/harry/wandr/swift/OpenSwiftUIProject/wandr/wasi-shims \
  -Xcc -include -Xcc /home/harry/wandr/swift/OpenSwiftUIProject/wandr/wasi-shims/wasi_compat.h \
  -Xcc -fno-exceptions -Xcc -DSWIFT_INLINE_NAMESPACE=__runtime \
  -Xlinker -z -Xlinker stack-size=8388608

# 2. wrap as a component via the preview1 adapter.
wasm-tools component new "$CORE" \
  --adapt "wasi_snapshot_preview1=$ADAPTER" \
  -o "$OUT"
wasm-tools validate "$OUT" && echo "component OK: $OUT ($(du -h "$OUT" | cut -f1))"

# 3. deploy to both dev hosts as ui.wasm.
cp "$OUT" "$LINUX_UI" && echo "deployed Linux: $LINUX_UI"
if [ -d "$(dirname "$WIN_UI")" ]; then
  cp "$OUT" "$WIN_UI" && echo "deployed Windows: $WIN_UI"
else
  echo "WARN: Windows dir missing, skipped: $WIN_UI"
fi

# 4. sync bundle assets (pre-decoded SFX WAVs — see Audio/, WandrAudioPlayer.swift) so a redeploy
# never silently drops them.
for ASSETS_DIR in "$(dirname "$LINUX_UI")/../assets" "$(dirname "$WIN_UI")/../assets"; do
  [ -d "$ASSETS_DIR" ] && cp -f Audio/*.wav "$ASSETS_DIR"/ && echo "synced audio assets: $ASSETS_DIR"
done
echo "DONE"
