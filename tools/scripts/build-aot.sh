#!/usr/bin/env bash
# AOT-compile compose-app.wasm → compose-app.cwasm for aarch64-linux-android.
# The .cwasm can be loaded on Android without W^X / JIT privileges.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
INPUT="${1:-$REPO_ROOT/compose-app.wasm}"
OUTPUT="${INPUT%.wasm}.cwasm"

echo "AOT compiling: $INPUT → $OUTPUT"
wasmtime compile \
    --target aarch64-linux-android \
    -W component-model \
    -W gc \
    -W function-references \
    -W exceptions \
    -o "$OUTPUT" \
    "$INPUT"

echo "Done: $(du -sh "$OUTPUT" | cut -f1)  $OUTPUT"
