#!/usr/bin/env bash
# Build the war.signal warpkg: compile the engine + ui components, WAC-plug the
# ui onto the engine (so the bundle is self-contained), and assemble the
# installable warpkg under build/. Pass --deploy to push + install + relaunch on
# a connected device (keeps /state so the link + history survive).
#
#   ./build.sh            # build the warpkg only
#   ./build.sh --deploy   # build, then install + relaunch on device
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROTOC="${PROTOC:-$HOME/tools/protoc/bin/protoc}"  # engine deps build protos
PKG="$HERE/build/war.signal.warpkg"

echo "▸ build engine (wasm32-wasip2)"
( cd "$HERE/engine" && PROTOC="$PROTOC" cargo build --target wasm32-wasip2 --release )
echo "▸ build ui (wasm32-wasip2)"
( cd "$HERE/ui" && cargo build --target wasm32-wasip2 --release )

echo "▸ wac plug ui ◁ engine"
mkdir -p "$PKG/components"
wac plug \
  "$HERE/ui/target/wasm32-wasip2/release/signal_ui.wasm" \
  --plug "$HERE/engine/target/wasm32-wasip2/release/signal_engine.wasm" \
  -o "$PKG/components/ui.wasm"
wasm-tools validate "$PKG/components/ui.wasm"
cp "$HERE/package.toml" "$PKG/package.toml"
echo "✓ warpkg: $PKG ($(du -h "$PKG/components/ui.wasm" | cut -f1))"

if [[ "${1:-}" == "--deploy" ]]; then
  DEV=/data/local/tmp/war.signal.warpkg
  HOST="LD_LIBRARY_PATH=/data/local/tmp WART_APPS_ROOT=/data/local/tmp/wart-apps /data/local/tmp/wart-host"
  AR="WART_APPS_ROOT=/data/local/tmp/wart-apps /data/local/tmp/wart-arbiter"
  echo "▸ push + install (AOT precompile)"
  adb shell "rm -rf $DEV" 2>&1 >/dev/null
  adb push "$PKG" "$DEV" >/dev/null
  adb shell "su -c '$HOST --install $DEV'" 2>&1 | tr -d '\r' | tail -1
  echo "▸ relaunch"
  adb shell "su -c '$AR kill war.signal'" 2>&1 | tr -d '\r' >/dev/null || true
  adb shell "su -c '$AR launch war.signal'" 2>&1 | tr -d '\r'
fi
