#!/usr/bin/env bash
# Republish the 9 compose modules listed in BUILD-wasmWasi.md Step 5
# ("MUST republish every compose module that consumed [skiko]").
# Run from compose-multiplatform-core repo using its in-tree gradle.

set -e

GRADLE_OPTS_WASI="-Dorg.gradle.configureondemand=false --console=plain --no-daemon --no-configuration-cache"

cd /home/harry/wandr/external/compose-multiplatform-core

# Dep order: leaf-first (graphics → text → ui → foundation-layout →
# foundation → animation-core → animation → material-ripple → material3).
PROJS=(
  :compose:ui:ui-graphics
  :compose:ui:ui-text
  :compose:ui:ui
  :compose:foundation:foundation-layout
  :compose:animation:animation-core
  :compose:animation:animation
  :compose:foundation:foundation
  :compose:material:material-ripple
  :compose:material3:material3
)

for proj in "${PROJS[@]}"; do
  echo ""
  echo "=========================================================="
  echo "$(date +%H:%M:%S)  rebuilding $proj"
  echo "=========================================================="
  ./gradlew "${proj}:publishWasmWasiPublicationToMavenLocal" $GRADLE_OPTS_WASI 2>&1 | tail -15
  rc=${PIPESTATUS[0]}
  if [[ $rc -ne 0 ]]; then
    echo "FAILED: $proj (rc=$rc)"
    exit $rc
  fi
done

echo ""
echo "=========================================================="
echo "$(date +%H:%M:%S)  all 9 compose modules republished"
echo "=========================================================="
