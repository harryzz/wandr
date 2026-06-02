#!/usr/bin/env bash
# Build the war.signal warpkg: compile the engine + ui components, WAC-plug the
# ui onto the engine (so the bundle is self-contained), and assemble the
# installable warpkg under build/. Pass --deploy to push + install + relaunch on
# a connected device. --deploy first BACKS UP the Signal /state (link + history +
# protocol store) on-device, then does a per-app install that preserves /state.
# Never use tools/scripts/build-system-warpkgs.sh for this — it wipes APPS_ROOT.
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
  APPS_ROOT=/data/local/tmp/wart-apps
  SIG_DIR="$APPS_ROOT/apps/war.signal"
  DEV=/data/local/tmp/war.signal.warpkg
  HOST="LD_LIBRARY_PATH=/data/local/tmp WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-host"
  AR="WART_APPS_ROOT=$APPS_ROOT /data/local/tmp/wart-arbiter"

  # ── SAFETY: back up the Signal /state (link + chat history + protocol store)
  # BEFORE touching anything. Per-app --install overwrites component files but
  # does NOT delete state/, so a normal redeploy already preserves it — this is
  # the explicit safety net against a bad install or a version bump orphaning
  # the old version's state. On-device copy (cheap); keeps the newest 5.
  # Restore:  adb shell "su -c 'cp -a <backup>/<ver>-state $SIG_DIR/<ver>/state'"
  STAMP="$(date +%Y%m%d-%H%M%S)"
  BACKUP="$APPS_ROOT/.signal-state-backups/$STAMP"
  echo "▸ backup Signal /state → device:$BACKUP (if present)"
  adb shell "su -c '
    if ls -d $SIG_DIR/*/state >/dev/null 2>&1; then
      mkdir -p $BACKUP || exit 1
      for s in $SIG_DIR/*/state; do
        v=\$(basename \$(dirname \"\$s\"))
        cp -a \"\$s\" \"$BACKUP/\$v-state\" || exit 1
      done
      du -sh $BACKUP 2>/dev/null | sed \"s|^|  backed up |\"
      ls -1dt $APPS_ROOT/.signal-state-backups/*/ 2>/dev/null | tail -n +6 | xargs -r rm -rf
    else
      echo \"  no existing Signal /state to back up (first install)\"
    fi
  '" 2>&1 | tr -d '\r'

  echo "▸ push + install (AOT precompile)"
  adb shell "rm -rf $DEV" 2>&1 >/dev/null
  adb push "$PKG" "$DEV" >/dev/null
  adb shell "su -c '$HOST --install $DEV'" 2>&1 | tr -d '\r' | tail -1
  echo "▸ relaunch"
  adb shell "su -c '$AR kill war.signal'" 2>&1 | tr -d '\r' >/dev/null || true
  adb shell "su -c '$AR launch war.signal'" 2>&1 | tr -d '\r'
fi
