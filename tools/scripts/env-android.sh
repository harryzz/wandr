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

_WANDR_TC="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
_WANDR_API="${ANDROID_API_LEVEL:-$ANDROID_API_DEFAULT}"

export CC_aarch64_linux_android="${CC_aarch64_linux_android:-$_WANDR_TC/aarch64-linux-android${_WANDR_API}-clang}"
export CXX_aarch64_linux_android="${CXX_aarch64_linux_android:-$_WANDR_TC/aarch64-linux-android${_WANDR_API}-clang++}"
export AR_aarch64_linux_android="${AR_aarch64_linux_android:-$_WANDR_TC/llvm-ar}"

# rustc's linker for the target. This used to live in wandr-host/.cargo/config.toml as an
# absolute path, which made that crate un-clonable once it became its own repo
# (github.com/harryzz/wandr-host) — a fresh checkout on any other machine pointed at
# /home/harry/android-ndk-r27d. It belongs here, with the rest of the NDK env, derived
# from $ANDROID_NDK_HOME. The versioned clang driver supplies its own sysroot and
# defaults to lld, so the old explicit --sysroot / -fuse-ld=lld link-args are not needed
# (and we must NOT set *_RUSTFLAGS here: that would clobber the aes_armv8 / polyval_armv8
# cfgs the crate's config.toml sets for the ARMv8 crypto backends).
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER:-$_WANDR_TC/aarch64-linux-android${_WANDR_API}-clang}"

case ":$PATH:" in
    *":$_WANDR_TC:"*) ;;
    *) export PATH="$_WANDR_TC:$PATH" ;;
esac

unset _WANDR_TC _WANDR_API NDK_HOME_DEFAULT SDK_HOME_DEFAULT ANDROID_API_DEFAULT

# Smoke-check the toolchain exists. Cheap; gives a clearer error than
# cc-rs's "tool not found" deep inside the build.
if [[ ! -x "$CC_aarch64_linux_android" ]]; then
    echo "env-android.sh: CC not executable: $CC_aarch64_linux_android" >&2
    echo "  hint: set ANDROID_API_LEVEL=NN to pick another versioned clang" >&2
    return 1 2>/dev/null || exit 1
fi
