#!/usr/bin/env bash
# Build the standalone sf_probe binary (task 33 Step 1, M5) for aarch64
# Android. Compiles wart-host/cpp/sf_probe.cpp against vendored AOSP headers
# and links against device-pulled .so libs in vendor/device-libs/.
#
# Output: /tmp/sf_probe   (push to /data/local/tmp and su-run on device)
set -euo pipefail

NDK="${ANDROID_NDK_HOME:-/home/harry/android-ndk-r27d}"
API=35
CXX="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android${API}-clang++"

WH="$(cd "$(dirname "$0")/../wart-host" && pwd)"
V="$WH/vendor"
FN="$V/aosp-frameworks-native"
SC="$V/aosp-system-core"
OUT="${OUT:-/tmp/sf_probe}"

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
