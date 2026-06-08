// audioclient-rs build.rs — codegen the `IAudioFlingerService` AIDL closure
// (IAudioTrack / IAudioRecord / CreateTrack*/CreateRecord* / SharedFileRegion).
// Mirrors wart-hal-sensors/build.rs. Runs only when cross-compiling for Android;
// off-android the crate is no-op stubs and needs no bindings.
//
// AIDL source: by default the device-matched AOSP submodule vendored under the
// wart-host crate (one copy, referenced by relative path). For a self-contained /
// publishable build, run ./vendor-aidl.sh to copy the minimal set into ./aidl/ —
// this build.rs prefers a local ./aidl/ dir if present.

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "android" {
        return;
    }

    use std::path::PathBuf;
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");

    // The binder runtime (frameworks-layer libbinder_ndk) for the final link.
    println!("cargo:rustc-link-lib=binder_ndk");

    // Prefer a self-contained local ./aidl/ (populated by vendor-aidl.sh); else
    // read the device-matched AOSP submodule vendored in the wart-host crate.
    let local = PathBuf::from("aidl");
    let vendor = if local.join("aosp-frameworks-av").is_dir() {
        local
    } else {
        PathBuf::from("../../runtime/wart-host/vendor")
    };
    println!("cargo:rerun-if-changed={}", vendor.display());

    let av = vendor.join("aosp-frameworks-av");
    let audioclient_aidl   = av.join("media/libaudioclient/aidl");
    let shmem_aidl         = av.join("media/libshmem/aidl");
    let av_aidl            = av.join("aidl");
    let audio_common_aidl  = vendor.join("aosp-system-hardware-interfaces/media/aidl");
    let stubs              = vendor.join("aidl-stubs");

    // rsbinder-aidl can't derive the *recursive* `AudioHalCapRule`
    // (`nestedRules: AudioHalCapRule[]` → Box recursion → no DeserializeArray; the
    // known rsbinder recursive-parcelable limitation). It's an audio-HAL config-engine
    // type pulled into the type closure, NOT on the createTrack/data path. Replace it
    // in place with a non-recursive stub (idempotent, self-healing across submodule
    // update) — same pattern as wart-host/wart-hal-sensors patch IDirectReportChannel.
    let cap_rule =
        audio_common_aidl.join("android/media/audio/common/AudioHalCapRule.aidl");
    if cap_rule.exists() {
        std::fs::write(
            &cap_rule,
            b"\
// Auto-patched by audioclient-rs/build.rs - the real AudioHalCapRule is recursive
// (nestedRules: AudioHalCapRule[]), which rsbinder-aidl can't derive. It's an
// audio-HAL config type not used on the createTrack path; stubbed non-recursive so
// the type closure compiles.
package android.media.audio.common;
@VintfStability
parcelable AudioHalCapRule {
    @VintfStability enum CompoundRule { INVALID = 0, ANY, ALL, }
    @VintfStability enum MatchingRule { INVALID = -1, IS = 0, IS_NOT, INCLUDES, EXCLUDES, }
    CompoundRule compoundRule = CompoundRule.INVALID;
}
",
        )
        .expect("patch AudioHalCapRule.aidl");
    }

    rsbinder_aidl::Builder::new()
        .source(audioclient_aidl.join("android/media/IAudioFlingerService.aidl"))
        .include_dir(audioclient_aidl)
        .include_dir(shmem_aidl)
        .include_dir(av_aidl)
        .include_dir(audio_common_aidl)
        .include_dir(stubs)
        .set_async_support(true)
        .output(PathBuf::from(&out_dir).join("audioflinger_bindings.rs"))
        .generate()
        .expect("rsbinder-aidl IAudioFlingerService codegen failed");
}
