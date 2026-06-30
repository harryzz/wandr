#!/usr/bin/env bash
# Run the 15-target oag-baseline suite on wasm and/or linux, capturing per-target pass/fail.
# Usage: bash run-suite.sh [wasm|linux|both]   (default: both)
# Builds must already exist (build-wasi.sh for wasm; `swift build` for linux).
set -u
cd "$(dirname "$0")"
MODE="${1:-both}"
TARGETS="oagattr oagbridge oagchurn oagcompare oagdataflow oagforeach oaggraph oagmemory oagrender oagrules oagsubgraph oagteardown oagupdate oagvalues oagweakref"
mkdir -p ../logs

run_wasm() {
  local LOG=../logs/suite-wasm-run.log; : > "$LOG"; local PASS=0 FAIL=0 FAILED=""
  for t in $TARGETS; do
    echo "===== $t =====" >> "$LOG"
    timeout 180 wasmtime run --env SWIFT_DETERMINISTIC_HASHING=1 -W max-wasm-stack=8388608 \
      ".build/wasm32-unknown-wasip1/debug/$t.wasm" >> "$LOG" 2>&1
    local rc=$?; echo "[$t] exit=$rc" >> "$LOG"
    if [ $rc -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); FAILED="$FAILED $t(exit=$rc)"; fi
  done
  echo "WASM SUITE: PASS=$PASS FAIL=$FAIL${FAILED:+  FAILED:$FAILED}" | tee -a "$LOG"
}

run_linux() {
  # libswiftDemangle.so lives in usr/lib (not usr/lib/swift/linux) under swiftly.
  local TC; TC=$(dirname "$(dirname "$(dirname "$(readlink -f "$(which swift)")")")")
  # readlink of swiftly shim -> swiftly bin; resolve real toolchain instead:
  local LIBP="/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib/swift/linux:/home/harry/.local/share/swiftly/toolchains/6.3.2/usr/lib"
  local LOG=../logs/suite-linux-run.log; : > "$LOG"; local PASS=0 FAIL=0 FAILED=""
  for t in $TARGETS; do
    echo "===== $t =====" >> "$LOG"
    LD_LIBRARY_PATH="$LIBP" SWIFT_DETERMINISTIC_HASHING=1 timeout 120 ".build/debug/$t" >> "$LOG" 2>&1
    local rc=$?; echo "[$t] exit=$rc" >> "$LOG"
    if [ $rc -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); FAILED="$FAILED $t(exit=$rc)"; fi
  done
  echo "LINUX SUITE: PASS=$PASS FAIL=$FAIL${FAILED:+  FAILED:$FAILED}" | tee -a "$LOG"
}

[ "$MODE" = "wasm" ] || [ "$MODE" = "both" ] && run_wasm
[ "$MODE" = "linux" ] || [ "$MODE" = "both" ] && run_linux
