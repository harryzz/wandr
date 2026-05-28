#!/usr/bin/env bash
# Generate the C++ AIDL headers that libgui's public headers (e.g.
# gui/SurfaceComposerClient.h) #include but which the AOSP build produces
# as build artifacts — they do not exist as source.
#
# Two backends:
#   - android.gui.*                 -> --lang=cpp -> android/gui/Foo.h
#   - android.hardware.graphics.common.* -> --lang=ndk -> aidl/android/hardware/graphics/common/Foo.h
#
# Output: wart-host/vendor/generated-aidl/{include,src}
# Part of task 33 Step 1.0 (libgui ABI de-risking). Re-runnable.
set -euo pipefail

VENDOR="$(cd "$(dirname "$0")/../wart-host/vendor" && pwd)"
FN="$VENDOR/aosp-frameworks-native"
HW="$VENDOR/aosp-hardware-interfaces"
OUT="$VENDOR/generated-aidl"
SCRATCH="$OUT/.aidl-src"
AIDL="${AIDL:-aidl}"

rm -rf "$OUT"
mkdir -p "$OUT/include" "$OUT/src" "$SCRATCH"

# The android.gui package is split across two source dirs in AOSP; merge
# both into one import root ($SCRATCH/root/android/gui). Extract PRISTINE
# AIDL from git, not the working tree — wart-host/build.rs mutates several
# gui AIDL files in place for rsbinder (ISurfaceComposer trimmed to 4
# methods, DisplayBrightness stubbed). The cpp aidl backend needs the
# originals. Also strip Android-15's `rust_type "..."` clause (the SDK-34
# `aidl` predates it; irrelevant to the cpp/ndk backends).
mkdir -p "$SCRATCH/root/android/gui" "$SCRATCH/extract"
git -C "$FN" archive HEAD libs/gui/aidl/android/gui libs/gui/android/gui \
    | tar -x -C "$SCRATCH/extract"
cp "$SCRATCH"/extract/libs/gui/aidl/android/gui/*.aidl "$SCRATCH/root/android/gui/"
cp "$SCRATCH"/extract/libs/gui/android/gui/*.aidl      "$SCRATCH/root/android/gui/"
find "$SCRATCH/root" -name '*.aidl' -exec \
    sed -i -E 's/ rust_type "[^"]*"//g' {} +

# No --structured for the gui package: it mixes structured parcelables with
# unstructured C++-backed ones (LayerMetadata, BitTube, ...). The real libgui
# build compiles it with plain aidl rules, not an aidl_interface module.
echo "== android.gui.* (cpp backend) =="
while IFS= read -r f; do
    "$AIDL" --lang=cpp "-I$SCRATCH/root" \
        -h "$OUT/include" -o "$OUT/src" "$f"
done < <(find "$SCRATCH/root/android/gui" -name '*.aidl' 2>/dev/null)

echo "== android.hardware.graphics.common.* (ndk backend) =="
GC="$HW/graphics/common/aidl"
while IFS= read -r f; do
    "$AIDL" --lang=ndk --structured --stability vintf \
        "-I$GC" "-I$HW/common/aidl" \
        -h "$OUT/include" -o "$OUT/src" "$f"
done < <(find "$GC/android" -name '*.aidl' 2>/dev/null)

echo "OK — headers in $OUT/include"
find "$OUT/include" -name '*.h' | wc -l | xargs echo "generated headers:"
