// wandr-sensors-client build.rs — codegen the `ISensorManager` AIDL closure (task 77).
//
// The sensors-only subset of wandr-host/build.rs's recipe. Reuses the AOSP AIDL that
// wandr-host vendors (one vendored copy, referenced by path) so there is no second set
// of multi-GB submodules. Runs only when cross-compiling for Android — off-android the
// crate is stubs and needs no bindings, so a desktop build never needs the AIDL at all.

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "android" {
        return;
    }

    use std::path::PathBuf;
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");

    // The binder runtime (frameworks-layer libbinder_ndk) for the final link.
    println!("cargo:rustc-link-lib=binder_ndk");

    // Locate wandr-host's vendored AOSP AIDL. This crate ships standalone
    // (github.com/harryzz/wandr-sensors-client) but is consumed from two different
    // layouts, so probe both rather than assuming one:
    //   monorepo   wandr/runtime/wandr-sensors-client  -> ../wandr-host/vendor
    //   submodule  wandr-host/crates/wandr-sensors-client -> ../../vendor
    // WANDR_AOSP_VENDOR overrides for anyone vendoring the AIDL elsewhere.
    println!("cargo:rerun-if-env-changed=WANDR_AOSP_VENDOR");
    let host_vendor = std::env::var("WANDR_AOSP_VENDOR")
        .map(PathBuf::from)
        .ok()
        .or_else(|| {
            ["../wandr-host/vendor", "../../vendor"]
                .iter()
                .map(PathBuf::from)
                .find(|p| p.join("aosp-frameworks-hardware-interfaces").is_dir())
        })
        .expect(
            "AOSP AIDL vendor dir not found. Android builds need wandr-host's vendored \
             AIDL — initialize the submodules (git submodule update --init --recursive) \
             or set WANDR_AOSP_VENDOR to a directory containing \
             aosp-hardware-interfaces/ and aosp-frameworks-hardware-interfaces/.",
        );
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
    // in place with a body-less interface (same patch wandr-host historically
    // applied; idempotent and self-healing across `git submodule update`).
    let direct_channel_path =
        sensorsvc_aidl.join("android/frameworks/sensorservice/IDirectReportChannel.aidl");
    let direct_channel_stub = b"\
// Auto-patched by wandr-sensors-client/build.rs because the real definition
// references android.hardware.sensors.ISensors.RateLevel which rsbinder-aidl
// doesn't resolve. We don't use direct channels.
package android.frameworks.sensorservice;
interface IDirectReportChannel {}
";
    std::fs::write(&direct_channel_path, direct_channel_stub).unwrap_or_else(|e| {
        panic!("patch IDirectReportChannel.aidl at {}: {e}", direct_channel_path.display())
    });

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
