// wart-hal-lights build.rs — codegen android.hardware.light.ILights (+ its
// HwLight/HwLightState/FlashMode/BrightnessMode/LightType deps, all plain AIDL
// parcelables/enums in the same package, resolved via include_dir). Sync client
// only (we only call out — getLights/setLightState — never register a callback), so
// no async/tokio. Android only; off-android the lib is a stub. Same AIDL the host
// codegens (wart-host/build.rs), pointed at the vendored hardware-interfaces tree.

use std::path::PathBuf;

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "android" {
        return;
    }

    println!("cargo:rustc-link-lib=binder_ndk");

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let light_aidl =
        PathBuf::from("../wart-host/vendor/aosp-hardware-interfaces/light/aidl");
    if !light_aidl.exists() {
        panic!("wart-hal-lights: AIDL source missing: {}", light_aidl.display());
    }

    rsbinder_aidl::Builder::new()
        .source(light_aidl.join("android/hardware/light/ILights.aidl"))
        .include_dir(light_aidl.clone())
        .output(out.join("light_bindings.rs"))
        .generate()
        .expect("rsbinder-aidl ILights codegen failed");

    println!("cargo:rerun-if-changed={}", light_aidl.display());
}
