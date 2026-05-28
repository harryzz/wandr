#!/usr/bin/env bash
# verify-libgui-abi.sh — task 33 Step 1.0 mitigation M4.
#
# An ELF built against AOSP platform libs (sf_probe, or the wart-host shim)
# links fine on the build host but must RUN against the device's own .so
# set. LineageOS could in principle diverge from the android-15.0.0_r36
# source the binary was built from. This catches that deterministically:
# every undefined symbol the ELF imports must be exported by one of the
# device-pulled libraries in wart-host/vendor/device-libs/.
#
# Usage: verify-libgui-abi.sh <path-to-aarch64-elf>
# Exit 0 = every imported symbol resolves on-device; non-zero = missing
# symbol(s) listed (a header/.so version mismatch — do not deploy).
set -euo pipefail

ELF="${1:?usage: verify-libgui-abi.sh <aarch64-elf>}"
NDK="${ANDROID_NDK_HOME:-/home/harry/android-ndk-r27d}"
NM="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-nm"
LIBS="$(cd "$(dirname "$0")/../../runtime/wart-host/vendor/device-libs" && pwd)"

[ -f "$ELF" ] || { echo "no such ELF: $ELF" >&2; exit 2; }
ls "$LIBS"/*.so >/dev/null 2>&1 || { echo "no device-libs in $LIBS" >&2; exit 2; }

# Exported symbols across every device .so (defined, dynamic).
exported="$(mktemp)"
for so in "$LIBS"/*.so; do
    "$NM" -D --defined-only "$so" 2>/dev/null | awk '{print $NF}'
done | sort -u > "$exported"

# Undefined (imported) symbols of the ELF.
undef="$("$NM" -D --undefined-only "$ELF" 2>/dev/null | awk '{print $NF}' | sort -u)"

missing=0
while IFS= read -r sym; do
    [ -z "$sym" ] && continue
    # Skip libc / libdl / libm / loader symbols — resolved by the platform
    # linker, never shipped in our device-libs snapshot.
    case "$sym" in
        __*|_ITM_*|abort|malloc|free|memcpy|memset|memmove|strlen|strcmp|\
        dlopen|dlsym|dlclose|dlerror|getenv|printf|puts|sleep|usleep|\
        pthread_*|sigaction|environ) continue ;;
    esac
    if ! grep -qxF "$sym" "$exported"; then
        echo "MISSING on-device: $sym"
        missing=$((missing + 1))
    fi
done <<< "$undef"

rm -f "$exported"
if [ "$missing" -gt 0 ]; then
    echo "FAIL — $missing symbol(s) not exported by device libs. Header/.so"
    echo "version mismatch: do NOT deploy. Re-pin vendored AOSP tag."
    exit 1
fi
echo "OK — every imported symbol resolves against device-libs/"
