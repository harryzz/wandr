#!/usr/bin/env bash
# Republish ALL ~30 wasi klibs per BUILD-wasmWasi.md Step 1 + 2 + 3.
#
# Use this when stdlib (or anything else everything depends on) changes.
# For an isolated skiko change, rebuild-compose-wasi-skiko-depend.sh is
# sufficient (just Step 3's skiko-consuming subset).
#
# Runs from compose-multiplatform-core repo using its in-tree gradle.
# End-to-end time: ~40-60 minutes on an 8-core workstation.

set -e

GRADLE_OPTS_WASI="-Dorg.gradle.configureondemand=false --console=plain --no-daemon --no-configuration-cache"

cd /home/harry/wart/external/compose-multiplatform-core

START_TS=$(date +%s)

run_step() {
    local label="$1"
    local proj="$2"
    local task="$3"
    echo ""
    echo "=========================================================="
    echo "$(date +%H:%M:%S)  [$label] $proj :: $task"
    echo "=========================================================="
    ./gradlew "${proj}:${task}" $GRADLE_OPTS_WASI 2>&1 | tail -15
    rc=${PIPESTATUS[0]}
    if [[ $rc -ne 0 ]]; then
        echo "FAILED: $proj :: $task (rc=$rc)"
        exit $rc
    fi
}

# ── Step 1.1 — 16 compatibility stubs (basic publish) ─────────────
# leaf-first ordering so transitives resolve as we go.
STUBS_BASIC=(
    :annotation:annotation
    :collection:collection
    :window:window-core
    :lifecycle:lifecycle-common
    :lifecycle:lifecycle-runtime
    :lifecycle:lifecycle-viewmodel
    :lifecycle:lifecycle-viewmodel-savedstate
    :savedstate:savedstate
    :navigation:navigation-common
    :navigation:navigation-runtime
    :compose:runtime:runtime
    :compose:runtime:runtime-saveable
    :lifecycle:lifecycle-runtime-compose
    :lifecycle:lifecycle-viewmodel-compose
    :savedstate:savedstate-compose
    :navigationevent:navigationevent-compose
)
for proj in "${STUBS_BASIC[@]}"; do
    run_step "Step1.1" "$proj" "publishWasmWasiPublicationToMavenLocal"
done

# ── Step 1.2 — 11 stubs that also need decorated umbrellas ────────
# These have downstream consumers that look up by
# `org.jetbrains.androidx.X:Y:9999.0.0-SNAPSHOT` umbrella coordinates.
STUBS_DECORATED=(
    :annotation:annotation
    :collection:collection
    :window:window-core
    :lifecycle:lifecycle-common
    :lifecycle:lifecycle-runtime
    :lifecycle:lifecycle-viewmodel
    :lifecycle:lifecycle-viewmodel-savedstate
    :savedstate:savedstate
    :navigation:navigation-common
    :navigation:navigation-runtime
    :compose:runtime:runtime
)
for proj in "${STUBS_DECORATED[@]}"; do
    run_step "Step1.2" "$proj" "publishKotlinMultiplatformDecoratedPublicationToMavenLocal"
done

# ── Step 2 — 2 supporting real-source modules ─────────────────────
# runtime-retain uses standard kotlinMultiplatform publication
# (no Decorated suffix) because no JetBrainsAndroidXPlugin applied.
# graphics-shapes has it applied and uses the decorated variant.
run_step "Step2"   :compose:runtime:runtime-retain  "publishWasmWasiPublicationToMavenLocal"
run_step "Step2"   :compose:runtime:runtime-retain  "publishKotlinMultiplatformPublicationToMavenLocal"
run_step "Step2"   :graphics:graphics-shapes        "publishWasmWasiPublicationToMavenLocal"
run_step "Step2"   :graphics:graphics-shapes        "publishKotlinMultiplatformDecoratedPublicationToMavenLocal"

# ── Step 3 — 13 real-source compose modules ──────────────────────
# Dep order: bottom-up.
COMPOSE_REAL=(
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
for proj in "${COMPOSE_REAL[@]}"; do
    run_step "Step3" "$proj" "publishWasmWasiPublicationToMavenLocal"
done

END_TS=$(date +%s)
ELAPSED=$((END_TS - START_TS))
MIN=$((ELAPSED / 60))
SEC=$((ELAPSED % 60))

echo ""
echo "=========================================================="
echo "$(date +%H:%M:%S)  all ~30 wasi klibs republished (${MIN}m ${SEC}s)"
echo "=========================================================="
