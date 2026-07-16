#!/usr/bin/env bash
# Cross-AOT device deploy for wandr.swiftui.demo, T2iles build (the REAL eleev/swiftui-2048 game,
# not the OpenSwiftUIDemo .onTapGesture demo — see swift/OpenSwiftUIProject/tests/deploy for that
# one). Same cross-AOT recipe: the component can't AOT on-device (OOM risk), compile the aarch64
# cwasm on the PC (WANDR_AOT_TARGET) and push it — the device just deserializes. Per-app install
# only (does NOT touch other device apps). Run build-t2iles.sh first.
set -euo pipefail
cd "$(dirname "$0")"
ROOT=/home/harry/wandr
# Debug is unusably slow on real device hardware — match build-t2iles.sh's release default.
CONFIG="${WANDR_T2ILES_CONFIG:-release}"
CORE=".build/wasm32-unknown-wasip1/$CONFIG/T2iles.wasm"
ADAPTER=$ROOT/external/skiko/wasi_snapshot_preview1.reactor.wasm
HOST=$ROOT/runtime/wandr-host/target/x86_64-unknown-linux-gnu/release/wasm-android-host
PKG=$ROOT/apps/user/wandr.swiftui.demo
STAGE=$PWD/stage-device
ID=wandr.swiftui.demo
VER=0.1.0
APPS=/data/local/tmp/wandr-apps/apps

echo "== 1. strip core (drop debug + name section for device) =="
wasm-tools strip "$CORE" -o /tmp/t2iles-core.wasm
wasm-tools strip --delete '^name$' /tmp/t2iles-core.wasm -o /tmp/t2iles-core2.wasm
ls -la /tmp/t2iles-core2.wasm

echo "== 2. component new -> pkg/components/ui.wasm =="
wasm-tools component new /tmp/t2iles-core2.wasm --adapt "wasi_snapshot_preview1=$ADAPTER" -o "$PKG/components/ui.wasm"
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
