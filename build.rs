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
        // submodule pinned to android-15.0.0_r36. Sparse-checkout limits
        // the working tree to vibrator/aidl, light/aidl, power/aidl, and
        // thermal/aidl (~700 KB). r36 is required for the stable AIDL
        // thermal HAL — earlier versions don't have it.
        use std::path::PathBuf;
        let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
        let vendor = PathBuf::from("vendor/aosp-hardware-interfaces");
        let vibrator_aidl = vendor.join("vibrator/aidl");
        let light_aidl    = vendor.join("light/aidl");
        let power_aidl    = vendor.join("power/aidl");
        let thermal_aidl  = vendor.join("thermal/aidl");
        let sensors_aidl  = vendor.join("sensors/aidl");
        let fmq_aidl      = vendor.join("common/fmq/aidl");
        let common_aidl   = vendor.join("common/aidl");
        // Frameworks-layer AIDL — separate submodule because it's a
        // different AOSP repo. Used by task 20 (ISensorManager etc).
        let fwk_vendor    = PathBuf::from("vendor/aosp-frameworks-hardware-interfaces");
        let sensorsvc_aidl = fwk_vendor.join("sensorservice/aidl");

        // Upstream IDirectReportChannel.aidl references
        // android.hardware.sensors.ISensors.RateLevel — a nested AIDL
        // enum that rsbinder-aidl 0.7.0 can't resolve. We don't use
        // direct channels in the WIT, so replace the file in place with
        // a body-less interface. Runs every build; cheap; self-healing
        // across `git submodule update`.
        let direct_channel_path = sensorsvc_aidl.join(
            "android/frameworks/sensorservice/IDirectReportChannel.aidl"
        );
        let direct_channel_stub = b"\
// Auto-patched by wart-host/build.rs because the real definition
// references android.hardware.sensors.ISensors.RateLevel which
// rsbinder-aidl 0.7.0 doesn't resolve. We don't use direct channels.
package android.frameworks.sensorservice;
interface IDirectReportChannel {}
";
        std::fs::write(&direct_channel_path, direct_channel_stub)
            .expect("patch IDirectReportChannel.aidl");
        // Framework-side AIDL types that hardware/interfaces depends on but
        // we don't vendor (the real ones live in frameworks/base). We provide
        // empty `parcelable Foo;` stubs that satisfy the import resolver but
        // are never actually constructed because the methods that use them
        // are not called from our host.
        let stubs = PathBuf::from("vendor/aidl-stubs");

        // ── AAudio AIDL (task 21) ────────────────────────────────────────
        // IAAudioService + supporting parcelables for PCM playback over the
        // `media.aaudio` binder service. The audio/common types
        // (AudioFormatDescription, AudioFormatType, PcmType, …) live in a
        // separate AOSP repo (system/hardware/interfaces) — that vendor is
        // pinned to android-15.0.0_r36 alongside the others.
        let aaudio_av  = PathBuf::from("vendor/aosp-frameworks-av");
        let aaudio_aidl = aaudio_av.join("media/libaaudio/src/binding/aidl");
        let shmem_aidl  = aaudio_av.join("media/libshmem/aidl");
        let audio_common_aidl = PathBuf::from(
            "vendor/aosp-system-hardware-interfaces/media/aidl"
        );

        // ── ISurfaceComposer AIDL (task 22) ──────────────────────────────
        // SurfaceFlingerAIDL service ("android.gui.ISurfaceComposer").
        // Parcelables live in two sibling dirs: most under libs/gui/aidl/,
        // plus a handful (IWindowInfosListener/Publisher,
        // StalledTransactionInfo, WindowInfo, FocusRequest, …) under
        // libs/gui/android/gui/. Both share package `android.gui` so we
        // include both. Zero imports leave the package. We only call
        // getPhysicalDisplayIds (read-only, no permission) for the §5
        // de-risk round-trip; the rest are emitted but unused.
        let surfaceflinger_aidl_main = PathBuf::from(
            "vendor/aosp-frameworks-native/libs/gui/aidl"
        );
        let surfaceflinger_aidl_extras = PathBuf::from(
            "vendor/aosp-frameworks-native/libs/gui"
        );

        // The upstream ISurfaceComposer.aidl is huge (100+ methods, many
        // referencing types backed by an external `gui_aidl_types_rs`
        // crate that we don't pull in, plus `IWindowInfosPublisher`
        // which lacks a `Default` impl). For the §5 de-risk we only call
        // getPhysicalDisplayIds — the 4th method (transaction code
        // FIRST_CALL_TRANSACTION + 3). Replace the file with a trimmed
        // version that preserves the first 4 method declarations (so
        // transaction codes match the service) and prunes the rest.
        // Self-heals on every build, survives `git submodule update`.
        let surface_composer_path = surfaceflinger_aidl_main
            .join("android/gui/ISurfaceComposer.aidl");
        let surface_composer_stub = b"\
// Auto-patched by wart-host/build.rs to keep only the first 4 methods
// of android.gui.ISurfaceComposer (so getPhysicalDisplayIds remains at
// FIRST_CALL_TRANSACTION + 3, matching the SurfaceFlingerAIDL service's
// wire protocol). The upstream interface references types
// (IWindowInfosPublisher, WindowInfo via gui_aidl_types_rs) that
// rsbinder-aidl 0.7.0 doesn't resolve. We only call
// getPhysicalDisplayIds (read-only, no permission); the other three
// methods are kept as declarations to preserve transaction codes but
// are never invoked.
package android.gui;

interface ISurfaceComposer {
    void bootFinished();
    @nullable IBinder createConnection();
    void destroyVirtualDisplay(IBinder displayToken);
    long[] getPhysicalDisplayIds();
}
";
        std::fs::write(&surface_composer_path, surface_composer_stub)
            .expect("patch ISurfaceComposer.aidl");

        // Pass only the interface .aidl files; parcelables/enums in the same
        // package are resolved automatically via include_dir. Passing the full
        // dir causes the package modules to be re-emitted once per file (~3×).
        rsbinder_aidl::Builder::new()
            .source(vibrator_aidl.join("android/hardware/vibrator/IVibrator.aidl"))
            .source(light_aidl.join("android/hardware/light/ILights.aidl"))
            .source(power_aidl.join("android/hardware/power/IPower.aidl"))
            .source(thermal_aidl.join("android/hardware/thermal/IThermal.aidl"))
            .source(sensorsvc_aidl.join("android/frameworks/sensorservice/ISensorManager.aidl"))
            .source(sensorsvc_aidl.join("android/frameworks/sensorservice/IEventQueue.aidl"))
            .source(sensorsvc_aidl.join("android/frameworks/sensorservice/IEventQueueCallback.aidl"))
            .source(aaudio_aidl.join("aaudio/IAAudioService.aidl"))
            .source(aaudio_aidl.join("aaudio/IAAudioClient.aidl"))
            .source(surfaceflinger_aidl_main.join("android/gui/ISurfaceComposer.aidl"))
            .include_dir(vibrator_aidl.clone())
            .include_dir(light_aidl.clone())
            .include_dir(power_aidl.clone())
            .include_dir(thermal_aidl.clone())
            .include_dir(sensors_aidl.clone())
            .include_dir(sensorsvc_aidl.clone())
            .include_dir(fmq_aidl.clone())
            .include_dir(common_aidl.clone())
            .include_dir(aaudio_aidl.clone())
            .include_dir(shmem_aidl.clone())
            .include_dir(audio_common_aidl.clone())
            .include_dir(surfaceflinger_aidl_main.clone())
            .include_dir(surfaceflinger_aidl_extras.clone())
            .include_dir(stubs.clone())
            .set_async_support(true)
            .output(PathBuf::from(&out_dir).join("aosp_hal_bindings.rs"))
            .generate()
            .expect("rsbinder-aidl codegen failed");

        println!("cargo:rerun-if-changed={}", vibrator_aidl.display());
        println!("cargo:rerun-if-changed={}", light_aidl.display());
        println!("cargo:rerun-if-changed={}", power_aidl.display());
        println!("cargo:rerun-if-changed={}", thermal_aidl.display());
        println!("cargo:rerun-if-changed={}", sensors_aidl.display());
        println!("cargo:rerun-if-changed={}", sensorsvc_aidl.display());
        println!("cargo:rerun-if-changed={}", fmq_aidl.display());
        println!("cargo:rerun-if-changed={}", common_aidl.display());
        println!("cargo:rerun-if-changed={}", aaudio_aidl.display());
        println!("cargo:rerun-if-changed={}", shmem_aidl.display());
        println!("cargo:rerun-if-changed={}", audio_common_aidl.display());
        println!("cargo:rerun-if-changed={}", surfaceflinger_aidl_main.display());
        println!("cargo:rerun-if-changed={}", surfaceflinger_aidl_extras.display());
        println!("cargo:rerun-if-changed={}", stubs.display());
    }

    println!("cargo:rerun-if-changed=build.rs");
}
