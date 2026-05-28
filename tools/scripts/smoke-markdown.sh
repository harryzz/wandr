#!/usr/bin/env bash
# Task 36 step 7 — full device end-to-end validation of cross-app dep
# wiring: install the markdown system bundle, install the smoke
# consumer (which declares a dependency on markdown), run `wart-host
# --run-once com.example.md-smoke`, observe logcat for the loader's
# dep-wiring log lines + a clean exit.
#
# Usage:
#   bash scripts/smoke-markdown.sh
#
# Steps:
#   1. Build markdown-renderer.wasm (wasm32-wasip2).
#   2. AOT-compile markdown-renderer for aarch64-android.
#   3. Build smoke consumer Kotlin/Wasm.
#   4. wasm-tools embed + new --adapt (command adapter) → consumer
#      component.wasm.
#   5. AOT-compile consumer for aarch64-android.
#   6. Push host binary + libsf_surface.so (skip if mtimes match).
#   7. Push two warpkg dirs to /data/local/tmp.
#   8. wart-host --install both packages (system bundle first; consumer
#      second so its resolver finds the dep).
#   9. wart-host --run-once com.example.md-smoke.
#   10. Print logcat tail (run_once: + loader: lines).
#
# Logs everything to /tmp/smoke-markdown-<timestamp>.log for post-mortem.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

LOG="/tmp/smoke-markdown-$(date +%Y%m%d-%H%M%S).log"
echo "▸ Logging to $LOG"
exec > >(tee -a "$LOG") 2>&1

# ── 1. markdown-renderer wasm ────────────────────────────────────────────
echo "▸ [1/10] Building markdown-renderer for wasm32-wasip2 …"
( cd "$REPO_ROOT/markdown-renderer" \
  && cargo build --target wasm32-wasip2 --release --quiet )
MD_WASM="$REPO_ROOT/markdown-renderer/target/wasm32-wasip2/release/markdown_renderer.wasm"
ls -la "$MD_WASM"

# ── 2. markdown-renderer cwasm (aarch64-android) ─────────────────────────
echo "▸ [2/10] AOT-compiling markdown-renderer for aarch64-android …"
MD_CWASM="/tmp/markdown_renderer.cwasm"
wasmtime compile \
    --target aarch64-linux-android \
    --wasm component-model --wasm gc --wasm function-references --wasm exceptions \
    -o "$MD_CWASM" "$MD_WASM"
ls -la "$MD_CWASM"

# ── 3. smoke consumer Kotlin/Wasm ────────────────────────────────────────
echo "▸ [3/10] Building wart-app-md-smoke (Kotlin/Wasm) …"
( cd "$REPO_ROOT/wart-app-md-smoke" \
  && ./gradlew compileProductionExecutableKotlinWasmWasi \
       --console=plain --no-daemon )
SMOKE_KOTLIN_WASM="$REPO_ROOT/wart-app-md-smoke/build/compileSync/wasmWasi/main/productionExecutable/kotlin/wart-app-md-smoke.wasm"
ls -la "$SMOKE_KOTLIN_WASM"

# ── 4. embed + adapt → component ─────────────────────────────────────────
echo "▸ [4/10] Embedding WIT + adapting with command adapter …"
SMOKE_EMBED="/tmp/md-smoke-embedded.wasm"
SMOKE_COMPONENT="/tmp/md-smoke-component.wasm"
ADAPTER="$REPO_ROOT/wasmtime-src/target/wasm32-unknown-unknown/release/wasi_snapshot_preview1.command.wasm"

if [[ ! -f "$ADAPTER" ]]; then
    echo "✗ command adapter missing: $ADAPTER"
    echo "  Build with: cd $REPO_ROOT/wasmtime-src && cargo build -p wasi-preview1-component-adapter --target wasm32-unknown-unknown --release --no-default-features --features command"
    exit 1
fi

wasm-tools component embed \
    --world md-smoke \
    "$REPO_ROOT/wart-app-md-smoke/wit" \
    "$SMOKE_KOTLIN_WASM" \
    -o "$SMOKE_EMBED"
wasm-tools component new "$SMOKE_EMBED" \
    --adapt "wasi_snapshot_preview1=$ADAPTER" \
    -o "$SMOKE_COMPONENT"
ls -la "$SMOKE_COMPONENT"

# ── 5. smoke consumer cwasm (aarch64-android) ────────────────────────────
echo "▸ [5/10] AOT-compiling smoke consumer for aarch64-android …"
SMOKE_CWASM="/tmp/md-smoke.cwasm"
wasmtime compile \
    --target aarch64-linux-android \
    --wasm component-model --wasm gc --wasm function-references --wasm exceptions \
    -o "$SMOKE_CWASM" "$SMOKE_COMPONENT"
ls -la "$SMOKE_CWASM"

# ── 6. push host binary if newer (wart-host already deployed from task 33) ─
echo "▸ [6/10] Pushing wart-host binary …"
HOST_BIN="$REPO_ROOT/wart-host/target/aarch64-linux-android/release/wasm-android-host"
adb push "$HOST_BIN" /data/local/tmp/wart-host >/dev/null
adb shell "chmod 0755 /data/local/tmp/wart-host"

# ── 7. construct warpkg dirs ─────────────────────────────────────────────
echo "▸ [7/10] Building warpkg dirs …"
MD_PKG="/tmp/markdown.warpkg"
SMOKE_PKG="/tmp/md-smoke.warpkg"

rm -rf "$MD_PKG" "$SMOKE_PKG"
mkdir -p "$MD_PKG/components" "$SMOKE_PKG/components"

cp "$MD_WASM" "$MD_PKG/components/renderer.wasm"
cat > "$MD_PKG/package.toml" <<'EOF'
app_id      = "war.markdown.renderer"
version     = "0.1.0"
world       = "war:markdown/renderer-world"
kind        = "system"
composition = "same-store"

[components]
renderer = "components/renderer.wasm"
EOF

cp "$SMOKE_COMPONENT" "$SMOKE_PKG/components/ui.wasm"
cat > "$SMOKE_PKG/package.toml" <<'EOF'
app_id      = "com.example.md-smoke"
version     = "0.0.1"
world       = "md-smoke"
composition = "same-store"

[components]
ui = "components/ui.wasm"

[dependencies]
markdown = { system = "war.markdown.renderer", version = "0.1.0", interface = "war:markdown/renderer@0.1.0" }
EOF

# Push both warpkgs.
adb push "$MD_PKG"    /data/local/tmp/markdown.warpkg >/dev/null
adb push "$SMOKE_PKG" /data/local/tmp/md-smoke.warpkg >/dev/null

# ── 8. install both packages on device ───────────────────────────────────
echo "▸ [8/10] Installing markdown system bundle + smoke consumer …"
APPS_ROOT="/data/local/tmp/wart-apps"
adb shell "su -c 'rm -rf $APPS_ROOT && mkdir -p $APPS_ROOT'"

WART_ENV="LD_LIBRARY_PATH=/data/local/tmp WART_APPS_ROOT=$APPS_ROOT"
set +e
echo "  - markdown.warpkg →"
adb shell "su -c '$WART_ENV /data/local/tmp/wart-host --install /data/local/tmp/markdown.warpkg'"
RC=$?
if [[ $RC -ne 0 ]]; then echo "✗ install markdown failed (rc=$RC)"; exit 1; fi

echo "  - md-smoke.warpkg →"
adb shell "su -c '$WART_ENV /data/local/tmp/wart-host --install /data/local/tmp/md-smoke.warpkg'"
RC=$?
if [[ $RC -ne 0 ]]; then echo "✗ install md-smoke failed (rc=$RC)"; exit 1; fi
set -e

echo "▸ On-device install layout:"
adb shell "ls -laR $APPS_ROOT" | head -60
echo
echo "▸ md-smoke cache-key.toml:"
adb shell "cat $APPS_ROOT/apps/com.example.md-smoke/0.0.1/cache-key.toml"

# ── 9. run smoke via --run-once ─────────────────────────────────────────
echo
echo "▸ [9/10] Starting logcat tail + running wart-host --run-once …"
adb logcat -c
adb logcat -v time wart-host:V wasm-android-host:V '*:S' > /tmp/smoke-markdown-logcat.log &
LOGCAT_PID=$!
trap "kill $LOGCAT_PID 2>/dev/null || true" EXIT

set +e
adb shell "su -c '$WART_ENV /data/local/tmp/wart-host --run-once com.example.md-smoke'"
RC=$?
set -e

# Give logcat a moment to drain.
sleep 1
kill $LOGCAT_PID 2>/dev/null || true
trap - EXIT

# ── 10. report ──────────────────────────────────────────────────────────
echo
echo "▸ [10/10] Result — run_once exit code: $RC"
echo
echo "── Captured logcat (loader: + run_once: lines) ──"
grep -E "loader:|run_once:|standalone:|install:" /tmp/smoke-markdown-logcat.log || echo "(no matching lines)"
echo
if [[ $RC -eq 0 ]]; then
    echo "✓ SUCCESS — wart-host --run-once com.example.md-smoke exited 0"
else
    echo "✗ FAILURE — wart-host --run-once exited $RC"
    echo
    echo "── Full logcat tail (last 80 lines) ──"
    tail -80 /tmp/smoke-markdown-logcat.log
fi
exit $RC
