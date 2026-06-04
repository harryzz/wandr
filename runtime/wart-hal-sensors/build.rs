// wart-hal-sensors build.rs — codegen the `ISensorManager` AIDL closure (task 77).
//
// The sensors-only subset of wart-host/build.rs's recipe. Reuses the AIDL
// vendored under wart-host/vendor (one vendored copy; referenced by relative
// path) so there is no second submodule. Runs only when cross-compiling for
// Android — off-android the crate is stubs and needs no bindings.

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "android" {
        return;
    }

    use std::path::PathBuf;
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");

    // The binder runtime (frameworks-layer libbinder_ndk) for the final link.
    println!("cargo:rustc-link-lib=binder_ndk");

    // Vendored AIDL lives in the sibling wart-host crate (one copy).
    let host_vendor = PathBuf::from("../wart-host/vendor");
    let hw_vendor = host_vendor.join("aosp-hardware-interfaces");
    let fwk_vendor = host_vendor.join("aosp-frameworks-hardware-interfaces");

    let sensors_aidl = hw_vendor.join("sensors/aidl");
    let fmq_aidl = hw_vendor.join("common/fmq/aidl");
    let common_aidl = hw_vendor.join("common/aidl");
    let sensorsvc_aidl = fwk_vendor.join("sensorservice/aidl");
    let stubs = host_vendor.join("aidl-stubs");

    // Upstream IDirectReportChannel.aidl references
    // android.hardware.sensors.ISensors.RateLevel — a nested AIDL enum that
    // rsbinder-aidl can't resolve. We don't use direct channels, so replace it
    // in place with a body-less interface (same patch wart-host historically
    // applied; idempotent and self-healing across `git submodule update`).
    let direct_channel_path =
        sensorsvc_aidl.join("android/frameworks/sensorservice/IDirectReportChannel.aidl");
    let direct_channel_stub = b"\
// Auto-patched by wart-hal-sensors/build.rs because the real definition
// references android.hardware.sensors.ISensors.RateLevel which rsbinder-aidl
// doesn't resolve. We don't use direct channels.
package android.frameworks.sensorservice;
interface IDirectReportChannel {}
";
    std::fs::write(&direct_channel_path, direct_channel_stub)
        .expect("patch IDirectReportChannel.aidl");

    rsbinder_aidl::Builder::new()
        .source(sensorsvc_aidl.join("android/frameworks/sensorservice/ISensorManager.aidl"))
        .source(sensorsvc_aidl.join("android/frameworks/sensorservice/IEventQueue.aidl"))
        .source(sensorsvc_aidl.join("android/frameworks/sensorservice/IEventQueueCallback.aidl"))
        .include_dir(sensors_aidl)
        .include_dir(sensorsvc_aidl)
        .include_dir(fmq_aidl)
        .include_dir(common_aidl)
        .include_dir(stubs)
        .set_async_support(true)
        .output(PathBuf::from(&out_dir).join("sensor_bindings.rs"))
        .generate()
        .expect("rsbinder-aidl sensors codegen failed");
}
