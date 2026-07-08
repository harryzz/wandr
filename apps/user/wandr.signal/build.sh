#!/usr/bin/env bash
# Build the wandr.signal wandrpkg: compile the engine + ui components, WAC-plug the
# ui onto the engine (so the bundle is self-contained), and assemble the
# installable wandrpkg under build/. Pass --deploy to push + install + relaunch on
# a connected device. --deploy first BACKS UP the Signal /state (link + history +
# protocol store) on-device, then does a per-app install that preserves /state.
# Never use tools/scripts/build-system-wandrpkgs.sh for this — it wipes APPS_ROOT.
#
#   ./build.sh            # build the wandrpkg only
#   ./build.sh --deploy   # build, then install + relaunch on device
#   P3=1 ./build.sh       # task 115: native CM-async flavor (engine built with
#                         # --features p3-async; wac compose re-exports the
#                         # host-called engine-start). Desktop p3-async host
#                         # only — NOT deployable until M4 (device cwasm hash).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROTOC="${PROTOC:-$HOME/tools/protoc/bin/protoc}"  # engine deps build protos
PKG="$HERE/build/wandr.signal.wandrpkg"
P3="${P3:-}"

if [[ -n "$P3" && "${1:-}" == "--deploy" ]]; then
  echo "ERROR: P3=1 --deploy is M4 (device host lacks p3-async; cwasm hash differs)" >&2
  exit 1
fi

echo "▸ build engine (wasm32-wasip2${P3:+, p3-async})"
( cd "$HERE/engine" && PROTOC="$PROTOC" cargo build --target wasm32-wasip2 --release ${P3:+--features p3-async} )
echo "▸ build ui (wasm32-wasip2)"
( cd "$HERE/ui" && cargo build --target wasm32-wasip2 --release )

mkdir -p "$PKG/components"
if [[ -n "$P3" ]]; then
  echo "▸ wac compose ui ◁ engine (+ engine-start re-export, p3)"
  wac compose \
    -d wandr:signal-engine="$HERE/engine/target/wasm32-wasip2/release/signal_engine.wasm" \
    -d wandr:signal-ui="$HERE/ui/target/wasm32-wasip2/release/signal_ui.wasm" \
    -o "$PKG/components/ui.wasm" \
    "$HERE/compose-p3.wac"
  # cm-async isn't in the validator's default feature set (wasm-tools 1.245).
  wasm-tools validate -f cm-async "$PKG/components/ui.wasm"
else
  echo "▸ wac plug ui ◁ engine"
  wac plug \
    "$HERE/ui/target/wasm32-wasip2/release/signal_ui.wasm" \
    --plug "$HERE/engine/target/wasm32-wasip2/release/signal_engine.wasm" \
    -o "$PKG/components/ui.wasm"
  wasm-tools validate "$PKG/components/ui.wasm"
fi
cp "$HERE/package.toml" "$PKG/package.toml"
echo "✓ wandrpkg: $PKG ($(du -h "$PKG/components/ui.wasm" | cut -f1))"

if [[ "${1:-}" == "--deploy" ]]; then
  APPS_ROOT=/data/local/tmp/wandr-apps
  SIG_DIR="$APPS_ROOT/apps/wandr.signal"
  DEV=/data/local/tmp/wandr.signal.wandrpkg
  HOST="LD_LIBRARY_PATH=/data/local/tmp WANDR_APPS_ROOT=$APPS_ROOT /data/local/tmp/wandr-host"
  AR="WANDR_APPS_ROOT=$APPS_ROOT /data/local/tmp/wandr-arbiter"

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
  adb shell "su -c '$AR kill wandr.signal'" 2>&1 | tr -d '\r' >/dev/null || true
  adb shell "su -c '$AR launch wandr.signal'" 2>&1 | tr -d '\r'
fi
