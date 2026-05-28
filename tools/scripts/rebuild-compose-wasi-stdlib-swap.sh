#!/usr/bin/env bash
# Rebuild ONLY the wasm-wasi klibs (publishWasmWasiPublication) for a
# stdlib swap.
#
# Unlike rebuild-compose-wasi-full.sh, this skips the umbrella /
# decorated / kotlinMultiplatform publishes — those produce variant-
# routing .module metadata that is version-pinned and unaffected by a
# stdlib change. They're only needed for a fresh-from-wipe bootstrap
# (BUILD-wasmWasi.md Step 6). For a stdlib swap, only the klibs (which
# recompile code against the stdlib) must be rebuilt.
#
# Note: the runtime-retain `publishKotlinMultiplatformPublication`
# umbrella task is also broken independently (compileCommonMainKotlinMetadata
# fails on Apple targets / unresolved compose symbols); skipping it here
# both speeds things up and dodges that pre-existing breakage.
#
# Runs from compose-multiplatform-core repo using its in-tree gradle.

set -e

GRADLE_OPTS_WASI="-Dorg.gradle.configureondemand=false --console=plain --no-daemon --no-configuration-cache"

cd /home/harry/wart/external/compose-multiplatform-core

START_TS=$(date +%s)

run_step() {
    local label="$1"
    local proj="$2"
    echo ""
    echo "=========================================================="
    echo "$(date +%H:%M:%S)  [$label] $proj :: publishWasmWasiPublicationToMavenLocal"
    echo "=========================================================="
    ./gradlew "${proj}:publishWasmWasiPublicationToMavenLocal" $GRADLE_OPTS_WASI 2>&1 | tail -15
    rc=${PIPESTATUS[0]}
    if [[ $rc -ne 0 ]]; then
        echo "FAILED: $proj (rc=$rc)"
        exit $rc
    fi
}

# All modules with a wasm-wasi target, leaf-first. Stubs first, then
# the 2 supporting modules, then the 13 real-source compose modules.
PROJS=(
    # Step 1 stubs — already republished against patched stdlib (commented out)
    # :annotation:annotation
    # :collection:collection
    # :window:window-core
    # :lifecycle:lifecycle-common
    # :lifecycle:lifecycle-runtime
    # :lifecycle:lifecycle-viewmodel
    # :lifecycle:lifecycle-viewmodel-savedstate
    # :savedstate:savedstate
    # :navigation:navigation-common
    # :navigation:navigation-runtime
    # :compose:runtime:runtime
    # :compose:runtime:runtime-saveable
    # :lifecycle:lifecycle-runtime-compose
    # :lifecycle:lifecycle-viewmodel-compose
    # :savedstate:savedstate-compose
    # :navigationevent:navigationevent-compose
    # Step 2 supporting modules — already republished (commented out)
    # :compose:runtime:runtime-retain
    # :graphics:graphics-shapes
    # Step 3 real-source compose modules
    :compose:ui:ui-util
    :compose:ui:ui-geometry
    :compose:ui:ui-unit
    :compose:ui:ui-graphics
    :compose:ui:ui-text
    :compose:ui:ui-backhandler
    :compose:ui:ui
    :compose:foundation:foundation-layout
    :compose:animation:animation-core
    :compose:animation:animation
    :compose:foundation:foundation
    :compose:material:material-ripple
    :compose:material3:material3
)

for proj in "${PROJS[@]}"; do
    run_step "wasi" "$proj"
done

END_TS=$(date +%s)
ELAPSED=$((END_TS - START_TS))
echo ""
echo "=========================================================="
echo "$(date +%H:%M:%S)  all 31 wasm-wasi klibs republished ($((ELAPSED/60))m $((ELAPSED%60))s)"
echo "=========================================================="
