#!/usr/bin/env bash
# Cross-AOT device deploy for wandr.calc (the andreiui/swift-calculator portability test). Same recipe
# as deploy-t2iles-device.sh: the component can't AOT on-device (OOM risk), so compile the aarch64
# cwasm on the PC (WANDR_AOT_TARGET) and push it — the device just deserializes. Per-app install only
# (does NOT touch other device apps). Run `WANDR_CALC_CONFIG=release ./build-calc.sh` first.
set -euo pipefail
cd "$(dirname "$0")"
ROOT=/home/harry/wandr
CONFIG="${WANDR_CALC_CONFIG:-release}"   # release: debug is unusably slow on device hardware
CORE=".build/wasm32-unknown-wasip1/$CONFIG/SwiftCalc.wasm"
ADAPTER=$ROOT/external/skiko/wasi_snapshot_preview1.reactor.wasm
HOST=$ROOT/runtime/wandr-host/target/x86_64-unknown-linux-gnu/release/wasm-android-host
PKG=$PWD/wandr.calc                       # source pkg (package.toml written by build-calc.sh)
STAGE=$PWD/stage-device-calc
ID=wandr.calc
VER=0.1.0
APPS=/data/local/tmp/wandr-apps/apps

echo "== 1. strip core (drop debug + name section for device) =="
wasm-tools strip "$CORE" -o /tmp/calc-core.wasm
wasm-tools strip --delete '^name$' /tmp/calc-core.wasm -o /tmp/calc-core2.wasm
ls -la /tmp/calc-core2.wasm

echo "== 2. component new -> pkg/components/ui.wasm =="
wasm-tools component new /tmp/calc-core2.wasm --adapt "wasi_snapshot_preview1=$ADAPTER" -o "$PKG/components/ui.wasm"
wasm-tools validate "$PKG/components/ui.wasm" && echo "component OK ($(du -h "$PKG/components/ui.wasm" | cut -f1))"

echo "== 3. cross-AOT install FOR aarch64-linux-android into local stage =="
rm -rf "$STAGE"
WANDR_AOT_TARGET=aarch64-linux-android WANDR_APPS_ROOT="$STAGE" "$HOST" --install "$PKG"
echo "staged:"; find "$STAGE/apps/$ID" -maxdepth 3

echo "== 4. push to device + launch =="
adb shell "su -c 'rm -rf $APPS/$ID'"
adb push "$STAGE/apps/$ID/$VER" "$APPS/$ID/$VER"
adb shell "su -c 'chmod -R 755 $APPS/$ID'"
echo "== launching $ID via the running arbiter daemon =="
adb shell "su -c '/data/local/tmp/wandr-arbiter launch $ID'"
echo "launch dispatched. Watch the device screen."
