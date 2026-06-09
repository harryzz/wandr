# Sourced helper — exports every env var the Android cross-build needs.
# Source from another script with: source "$(dirname "$0")/env-android.sh"
#
# Rationale: NDK r27 ships only API-versioned `aarch64-linux-androidNN-clang`
# binaries (no unversioned `aarch64-linux-android-clang`). cc-rs (used by
# skia-bindings, zstd-sys, others) defaults to the unversioned name and
# fails to find a tool. We pin to API 30 (matches the linker in
# wandr-host/.cargo/config.toml). `cargo apk` also wants ANDROID_HOME for
# the SDK platform jars + zipalign / apksigner.

NDK_HOME_DEFAULT=/home/harry/android-ndk-r27d
SDK_HOME_DEFAULT=/home/harry/android-sdk
ANDROID_API_DEFAULT=30

export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$NDK_HOME_DEFAULT}"
export ANDROID_NDK_ROOT="${ANDROID_NDK_ROOT:-$ANDROID_NDK_HOME}"
export ANDROID_HOME="${ANDROID_HOME:-$SDK_HOME_DEFAULT}"

_WART_TC="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
_WART_API="${ANDROID_API_LEVEL:-$ANDROID_API_DEFAULT}"

export CC_aarch64_linux_android="${CC_aarch64_linux_android:-$_WART_TC/aarch64-linux-android${_WART_API}-clang}"
export CXX_aarch64_linux_android="${CXX_aarch64_linux_android:-$_WART_TC/aarch64-linux-android${_WART_API}-clang++}"
export AR_aarch64_linux_android="${AR_aarch64_linux_android:-$_WART_TC/llvm-ar}"

case ":$PATH:" in
    *":$_WART_TC:"*) ;;
    *) export PATH="$_WART_TC:$PATH" ;;
esac

unset _WART_TC _WART_API NDK_HOME_DEFAULT SDK_HOME_DEFAULT ANDROID_API_DEFAULT

# Smoke-check the toolchain exists. Cheap; gives a clearer error than
# cc-rs's "tool not found" deep inside the build.
if [[ ! -x "$CC_aarch64_linux_android" ]]; then
    echo "env-android.sh: CC not executable: $CC_aarch64_linux_android" >&2
    echo "  hint: set ANDROID_API_LEVEL=NN to pick another versioned clang" >&2
    return 1 2>/dev/null || exit 1
fi
