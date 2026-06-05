// wart-hal-net build.rs — codegen a minimal `android.net.IDnsResolver` client
// (task 88, M1b — DNS via the dnsresolver binder, which has no `ndc` command).
//
// Android-only: off-android the crate is pure Rust (DHCP/supplicant/ip). We don't
// codegen the *full* IDnsResolver: several of its methods (`getResolverInfo` with
// many `out` arrays, the `register*Listener` callbacks) make rsbinder-aidl
// mis-generate the interface in sync mode. AIDL transaction codes are positional
// (FIRST_CALL_TRANSACTION + declaration index), so we emit a trimmed interface
// into OUT_DIR that preserves the method *order* up to the ones we call — real
// signatures for `isAlive` (0) / `setResolverConfiguration` (2) /
// `createNetworkCache` (7), trivial placeholders for the others, and nothing
// after — which keeps our three transaction codes identical to the device's
// while sidestepping the unused methods. The real parcelables are copied verbatim
// (the wart-hal-display "mutate in OUT_DIR, never the submodule" pattern).

use std::path::{Path, PathBuf};

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "android" {
        return;
    }
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR not set"));
    println!("cargo:rustc-link-lib=binder_ndk");

    let src = PathBuf::from("../wart-host/vendor/aosp-packages-modules-dnsresolver/binder");
    println!("cargo:rerun-if-changed={}", src.display());

    let aidl_root = out_dir.join("aidl");
    let net_dir = aidl_root.join("android/net");
    let resolv_dir = net_dir.join("resolv/aidl");
    std::fs::create_dir_all(&resolv_dir).unwrap();

    // Real parcelables (the closure of ResolverParamsParcel), copied verbatim.
    copy(&src.join("android/net/ResolverParamsParcel.aidl"), &net_dir.join("ResolverParamsParcel.aidl"));
    copy(&src.join("android/net/ResolverOptionsParcel.aidl"), &net_dir.join("ResolverOptionsParcel.aidl"));
    copy(&src.join("android/net/ResolverHostsParcel.aidl"), &net_dir.join("ResolverHostsParcel.aidl"));
    copy(&src.join("android/net/resolv/aidl/DohParamsParcel.aidl"), &resolv_dir.join("DohParamsParcel.aidl"));

    // Trimmed interface — methods 0..=7 in declaration order (codes preserved).
    std::fs::write(
        net_dir.join("IDnsResolver.aidl"),
        r#"// Trimmed by wart-hal-net build.rs — see build.rs header. Method ORDER is
// load-bearing (transaction codes); real signatures only for the calls we make.
package android.net;
import android.net.ResolverParamsParcel;
interface IDnsResolver {
    boolean isAlive();                                                  // code 0
    void registerEventListener(int unused);                            // code 1 (placeholder)
    void setResolverConfiguration(in ResolverParamsParcel resolverParams); // code 2
    void getResolverInfo(int netId);                                   // code 3 (placeholder)
    void startPrefix64Discovery(int netId);                            // code 4 (placeholder)
    void stopPrefix64Discovery(int netId);                             // code 5 (placeholder)
    @utf8InCpp String getPrefix64(int netId);                          // code 6 (placeholder)
    void createNetworkCache(int netId);                                // code 7
}
"#,
    )
    .unwrap();

    rsbinder_aidl::Builder::new()
        .source(net_dir.join("IDnsResolver.aidl"))
        .include_dir(&aidl_root)
        // Sync-only codegen is broken in this rsbinder (BnX never emitted); async
        // mode works. We still call the proxy synchronously — no callbacks here.
        .set_async_support(true)
        .output(out_dir.join("dns_bindings.rs"))
        .generate()
        .expect("rsbinder-aidl IDnsResolver codegen failed");

    // android.net.INetd — the full real interface (network create / interface /
    // route / default). Unlike IDnsResolver, the whole AIDL parses, generates,
    // and compiles in async mode (its callback interface is dyn-compatible), so
    // no trimming/stubbing is needed — codegen the vendored source verbatim.
    let conn = PathBuf::from(
        "../wart-host/vendor/aosp-packages-modules-connectivity/staticlibs/netd/binder",
    );
    println!("cargo:rerun-if-changed={}", conn.display());
    rsbinder_aidl::Builder::new()
        .source(conn.join("android/net/INetd.aidl"))
        .include_dir(&conn)
        .set_async_support(true)
        .output(out_dir.join("inetd_bindings.rs"))
        .generate()
        .expect("rsbinder-aidl INetd codegen failed");
}

fn copy(src: &Path, dst: &Path) {
    std::fs::copy(src, dst)
        .unwrap_or_else(|e| panic!("copy {} -> {}: {e}", src.display(), dst.display()));
}
