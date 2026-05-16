fn main() {
    let target_os   = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    // ── WasiDrawable C++ shim ────────────────────────────────────────────────
    // A small SkDrawable subclass with a *mutable* sk_sp<SkPicture> field so
    // child layers can swap their picture without invalidating parent
    // recordings that captured `drawDrawable(this)`. Headers are vendored
    // under host/vendor/skia-src/ at the skia-bindings 0.93.1 commit.
    //
    // We compile against vendored skia headers and let the linker resolve
    // SkDrawable / SkCanvas / SkPicture symbols against libskia.a, which
    // skia-bindings already pulls in.
    let skia_include = "vendor/skia-src";

    let mut cc = cc::Build::new();
    cc.cpp(true)
        .file("cpp/wasi_drawable.cpp")
        .include(skia_include)
        .flag_if_supported("-std=c++17")
        .flag_if_supported("-fno-exceptions")
        .flag_if_supported("-fno-rtti");

    if target_os == "android" {
        // Skia on Android is built with libc++. Match that.
        cc.cpp_set_stdlib(Some("c++"));
        // Skia uses these macros for Android builds; mismatching can cause
        // ABI differences in inline methods.
        cc.define("SK_BUILD_FOR_ANDROID", None);

        // cc-rs looks for `aarch64-linux-android-clang++` by default, but
        // NDK r23+ only ships versioned variants (e.g. android35-clang++).
        // Pick the API level via env or default to 35 (matches sysroot lib
        // dir below).
        let ndk = std::env::var("ANDROID_NDK_HOME")
            .or_else(|_| std::env::var("NDK_HOME"))
            .expect("ANDROID_NDK_HOME must be set when cross-compiling for Android");
        let api: u32 = std::env::var("ANDROID_PLATFORM")
            .ok()
            .and_then(|s| s.strip_prefix("android-").map(|x| x.to_string()).or(Some(s)))
            .and_then(|s| s.parse().ok())
            .unwrap_or(35);
        let triple = match target_arch.as_str() {
            "aarch64" => "aarch64-linux-android",
            "x86_64"  => "x86_64-linux-android",
            other     => panic!("unsupported Android arch: {other}"),
        };
        let toolchain_bin = format!(
            "{ndk}/toolchains/llvm/prebuilt/linux-x86_64/bin"
        );
        let cxx = format!("{toolchain_bin}/{triple}{api}-clang++");
        let ar  = format!("{toolchain_bin}/llvm-ar");
        cc.compiler(&cxx);
        cc.archiver(&ar);
    }

    cc.compile("wasi_drawable");

    println!("cargo:rerun-if-changed=cpp/wasi_drawable.cpp");
    println!("cargo:rerun-if-changed=cpp/wasi_drawable.h");

    // ── Android sysroot link config (unchanged) ──────────────────────────────
    if target_os == "android" {
        let ndk = std::env::var("ANDROID_NDK_HOME")
            .or_else(|_| std::env::var("NDK_HOME"))
            .expect("ANDROID_NDK_HOME must be set when cross-compiling for Android");

        let api = 35;
        let triple = match target_arch.as_str() {
            "aarch64" => "aarch64-linux-android",
            "x86_64"  => "x86_64-linux-android",
            other     => panic!("unsupported Android arch: {other}"),
        };
        let sysroot_lib = format!(
            "{ndk}/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/{triple}/{api}"
        );

        println!("cargo:rustc-link-search={sysroot_lib}");
        println!("cargo:rustc-link-lib=EGL");
        println!("cargo:rustc-link-lib=android");
        println!("cargo:rustc-link-lib=log");
        println!("cargo:rustc-link-lib=GLESv2");
        println!("cargo:rustc-link-lib=dl");
        println!("cargo:rustc-link-lib=binder_ndk");

        // ── rsbinder-aidl codegen for vendored AOSP HALs ─────────────────────
        // Vendored under vendor/aosp-hardware-interfaces/ as a shallow
        // submodule pinned to android-11.0.0_r48. Sparse-checkout limits
        // the working tree to vibrator/aidl and light/aidl (~544 KB).
        use std::path::PathBuf;
        let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
        let vendor = PathBuf::from("vendor/aosp-hardware-interfaces");
        let vibrator_aidl = vendor.join("vibrator/aidl");
        let light_aidl    = vendor.join("light/aidl");
        // Pass only the interface .aidl files; parcelables/enums in the same
        // package are resolved automatically via include_dir. Passing the full
        // dir causes the package modules to be re-emitted once per file (~3×).
        rsbinder_aidl::Builder::new()
            .source(vibrator_aidl.join("android/hardware/vibrator/IVibrator.aidl"))
            .source(light_aidl.join("android/hardware/light/ILights.aidl"))
            .include_dir(vibrator_aidl.clone())
            .include_dir(light_aidl.clone())
            .set_async_support(true)
            .output(PathBuf::from(&out_dir).join("aosp_hal_bindings.rs"))
            .generate()
            .expect("rsbinder-aidl codegen failed");

        println!("cargo:rerun-if-changed={}", vibrator_aidl.display());
        println!("cargo:rerun-if-changed={}", light_aidl.display());
    }

    println!("cargo:rerun-if-changed=build.rs");
}
