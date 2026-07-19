#!/usr/bin/env bash
# [wandr portability test] Build the andreiui/swift-calculator (target SwiftCalc) to a wasip1 reactor
# component + deploy as wandr.calc. Mirrors build-t2iles.sh but --product SwiftCalc, no audio assets.
set -euo pipefail
cd "$(dirname "$0")"
SDK="${SWIFT_WASM_SDK:-swift-6.3.2-RELEASE_wasm}"
ADAPTER=../../external/skiko/wasi_snapshot_preview1.reactor.wasm
CONFIG="${WANDR_CALC_CONFIG:-debug}"
CORE=".build/wasm32-unknown-wasip1/$CONFIG/SwiftCalc.wasm"
OUT=calc.component.wasm
APPROOT_LINUX=/home/harry/wandr-desktop-apps/apps/wandr.calc/0.1.0
LINUX_UI=$APPROOT_LINUX/components/ui.wasm

echo "== 1. SwiftPM cross-build --product SwiftCalc =="
OPENSWIFTUI_ANY_ATTRIBUTE_FIX=0 ANY_ATTRIBUTE_FIX=0 \
OPENSWIFTUI_USE_LOCAL_DEPS=1 OPENATTRIBUTEGRAPH_OPENATTRIBUTESHIMS_COMPUTE=1 \
OPENATTRIBUTEGRAPH_USE_LOCAL_DEPS=1 OPENSWIFTUI_SWIFT_CRYPTO=0 \
OPENRENDERBOX_LIB_SWIFT_PATH=/home/harry/wandr/swift/OpenSwiftUIProject/OpenAttributeGraph/Sources/SwiftCorelibs/include \
swift build --product SwiftCalc --swift-sdk "$SDK" --manifest-cache none -c "$CONFIG" \
  -Xswiftc -enforce-exclusivity=unchecked \
  -Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN -Xcc -D_WASI_EMULATED_PROCESS_CLOCKS \
  -Xcc -I/home/harry/wandr/swift/OpenSwiftUIProject/wandr/wasi-shims \
  -Xcc -include -Xcc /home/harry/wandr/swift/OpenSwiftUIProject/wandr/wasi-shims/wasi_compat.h \
  -Xcc -fno-exceptions -Xcc -DSWIFT_INLINE_NAMESPACE=__runtime \
  -Xlinker -z -Xlinker stack-size=8388608

echo "== 2. wrap as component =="
wasm-tools component new "$CORE" --adapt "wasi_snapshot_preview1=$ADAPTER" -o "$OUT"
wasm-tools validate "$OUT" && echo "component OK: $OUT ($(du -h "$OUT" | cut -f1))"

echo "== 3. build PKG + --install into desktop apps root (precompile -> cache-key.toml) =="
HOST="${WANDR_HOST:-$HOME/wandr/runtime/wandr-host/target/x86_64-unknown-linux-gnu/release/wasm-android-host}"
PKG="$PWD/wandr.calc"          # source pkg dir (package.toml + components/ui.wasm)
mkdir -p "$PKG/components"
# package.toml: clone swiftui.demo's world/composition, swap the id/label.
cat > "$PKG/package.toml" <<'TOML'
# andreiui/swift-calculator ported to OpenSwiftUI-on-wandr (portability test). components/ui.wasm is a
# regenerable build output (git-ignored) — rebuild via repros/swift-canvas-spike/build-calc.sh.
app_id      = "wandr.calc"
version     = "0.1.0"
world       = "my:skiko-gfx/skiko-ui"
composition = "same-store"
orientation = "auto"
label       = "SwiftUI Calculator"
max_fps     = 60

[components]
ui = "components/ui.wasm"
TOML
cp "$OUT" "$PKG/components/ui.wasm"
# JIT desktop install (no WANDR_AOT_TARGET): precompiles the component + writes cache-key.toml.
rm -rf "$APPROOT_LINUX"
WANDR_APPS_ROOT="$HOME/wandr-desktop-apps" "$HOST" --install "$PKG"
echo "installed:"; ls -la "$APPROOT_LINUX" 2>/dev/null
echo "DONE"
