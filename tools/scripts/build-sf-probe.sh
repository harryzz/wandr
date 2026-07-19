#!/usr/bin/env bash
# Build the standalone sf_probe binary (task 33 Step 1, M5) for aarch64
# Android. Compiles wandr-host/cpp/sf_probe.cpp against vendored AOSP headers
# and links against device-pulled .so libs in vendor/device-libs/.
#
# Output: /tmp/sf_probe   (push to /data/local/tmp and su-run on device)
set -euo pipefail

NDK="${ANDROID_NDK_HOME:-/home/harry/android-ndk-r27d}"
API=35
CXX="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android${API}-clang++"

WH="$(cd "$(dirname "$0")/../wandr-host" && pwd)"
V="$WH/vendor"
FN="$V/aosp-frameworks-native"
SC="$V/aosp-system-core"
OUT="${OUT:-/tmp/sf_probe}"

# ── vendored AOSP sources ────────────────────────────────────────────────────
# Everything below is .gitignore'd in wandr-host (see its .gitignore) and so is
# absent from a fresh checkout. Rather than failing with a wall of missing-header
# errors, fetch/regenerate what we can and give an exact command for what we
# shouldn't (the AOSP submodules are multi-GB — cloning them behind your back is
# not friendly).
AOSP_TAG=android-15.0.0_r36

# Headers-only clones, pinned to the tag the existing checkouts use.
ensure_aosp() { # <vendor-dir> <googlesource-platform-path>
    local dir="$V/$1"
    [ -d "$dir/.git" ] && return 0
    echo "== vendoring $1 @ $AOSP_TAG (headers only) =="
    git clone --quiet --depth 1 --branch "$AOSP_TAG" \
        "https://android.googlesource.com/platform/$2" "$dir"
}
ensure_aosp aosp-system-core    system/core
ensure_aosp aosp-system-libbase system/libbase
ensure_aosp aosp-system-logging system/logging

# Build output of gen-libgui-aidl.sh (the C++ headers AOSP's build generates from
# AIDL; they don't exist as source). Re-runnable.
if [ ! -d "$V/generated-aidl/include" ]; then
    echo "== generating libgui AIDL headers =="
    "$(dirname "$0")/gen-libgui-aidl.sh"
fi

# The big AOSP trees are submodules of wandr-host — instruct, don't auto-clone.
if [ ! -d "$FN/libs/gui/include" ]; then
    cat >&2 <<EOF
error: $FN is empty (wandr-host's vendor submodules are not initialized).
Run:
  git -C "$WH" submodule update --init --recursive \\
      vendor/aosp-frameworks-native vendor/aosp-hardware-interfaces
EOF
    exit 1
fi

INCLUDES=(
    "-I$FN/libs/gui/include"
    "-I$FN/libs/ui/include"
    "-I$FN/libs/binder/include"
    "-I$FN/libs/math/include"
    "-I$FN/libs/nativewindow/include"
    "-I$FN/libs/arect/include"
    "-I$FN/libs/nativebase/include"
    "-I$SC/libutils/include"
    "-I$SC/libcutils/include"
    "-I$SC/libsystem/include"
    "-I$V/aosp-system-logging/liblog/include"
    "-I$V/aosp-system-libbase/include"
    "-I$V/generated-aidl/include"
)

"$CXX" -std=c++20 -fPIE -pie -O1 -g \
    "${INCLUDES[@]}" \
    "$WH/cpp/sf_probe.cpp" \
    -L"$V/device-libs" \
    -lgui -lui -lutils -lbinder -lcutils -llog -lnativewindow -lEGL -lGLESv2 \
    -Wl,--allow-shlib-undefined \
    -o "$OUT"

echo "OK — $OUT"
file "$OUT"
