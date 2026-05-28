#!/usr/bin/env bash
# Build the wart-host APK for aarch64-android (release) and report the path.
#
# Pairs with scripts/build-host-android.sh which produces only the host
# binary (for the bare-binary deploy.sh flow). This one produces the
# signed APK that the Android side launches as NativeActivity — the
# default device flow per CLAUDE.md.
#
# Quirk: cargo-apk 0.12.0 panics with "Bin is not compatible with Cdylib"
# AFTER the APK has been built + signed (it then tries to also handle the
# [[bin]] entry as a second artifact and trips an internal assertion).
# The APK on disk is valid; the script treats panic-after-signing as a
# warning, not a failure.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=./env-android.sh
source "$REPO_ROOT/tools/scripts/env-android.sh"

cd "$REPO_ROOT/runtime/wart-host"

APK_PATH="$REPO_ROOT/runtime/wart-host/target/release/apk/wasm_android_host.apk"
APK_BEFORE_MTIME=0
if [[ -f "$APK_PATH" ]]; then
    APK_BEFORE_MTIME=$(stat -c '%Y' "$APK_PATH" 2>/dev/null || stat -f '%m' "$APK_PATH")
fi

echo "Building APK for aarch64-linux-android …"
BUILD_LOG=$(mktemp)
trap 'rm -f "$BUILD_LOG"' EXIT
cargo apk build --release 2>&1 | tee "$BUILD_LOG"
RC=${PIPESTATUS[0]}

# Treat the known cargo-apk panic-after-signing as success if the APK was
# refreshed: if the panic message appears AND the APK mtime is newer than
# when we started, the artifact is good.
if [[ $RC -ne 0 ]]; then
    if grep -q "Bin is not compatible with Cdylib" "$BUILD_LOG" \
        && [[ -f "$APK_PATH" ]]; then
        APK_AFTER_MTIME=$(stat -c '%Y' "$APK_PATH" 2>/dev/null || stat -f '%m' "$APK_PATH")
        if (( APK_AFTER_MTIME > APK_BEFORE_MTIME )); then
            echo
            echo "Note: cargo-apk panicked AFTER signing the APK (known 0.12.0 bug)."
            echo "      Artifact is valid; treating as success."
            RC=0
        fi
    fi
fi

if [[ $RC -ne 0 ]]; then
    echo "build-apk.sh: cargo apk build failed (rc=$RC)" >&2
    exit "$RC"
fi

echo "Built: $(du -sh "$APK_PATH" | cut -f1)  $APK_PATH"
echo
echo "Install with:"
echo "  adb install -r '$APK_PATH'"
