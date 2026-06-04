// Codegen the minimal android.app.IActivityManager from our local AIDL via
// rsbinder-aidl (host-side; no a-03). Produces both the Bp proxy and the Bn server
// trait; we implement the Bn (stub) in main.rs. Link binder_ndk on android.
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let gen = out.join("activitymanager.rs");
    rsbinder_aidl::Builder::new()
        .source(PathBuf::from("aidl/android/app/IActivityManager.aidl"))
        .output(gen.clone())
        .set_async_support(true)
        .generate()
        .expect("rsbinder-aidl IActivityManager codegen failed");

    // Force the served binder to Stability::Local. rsbinder-aidl emits
    // `Binder::new_with_stability(.., Stability::default())`, and rsbinder's default
    // is `System` — which the real Android-15 servicemanager treats as needing a
    // VINTF declaration (meetsDeclarationRequirements), so addService("activity")
    // fails with FailedTransaction. C++ libbinder registers a plain service as
    // `Local` (=0, no declaration requirement); match that. (Post-codegen string
    // rewrite, same approach as wart-hal-display's AIDL float-literal fix — avoids
    // forking rsbinder / rsbinder-aidl.)
    let src = std::fs::read_to_string(&gen).expect("read generated bindings");
    let patched = src.replace("rsbinder::Stability::default()", "rsbinder::Stability::Local");
    assert!(patched != src, "expected to rewrite Stability::default() → Local (codegen changed?)");
    std::fs::write(&gen, patched).expect("write patched bindings");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-lib=binder_ndk");
    }
    println!("cargo:rerun-if-changed=aidl/android/app/IActivityManager.aidl");
}
