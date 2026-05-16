extern crate wasm_android_host;

#[cfg(not(target_os = "android"))]
fn main() {
    wasm_android_host::run();
}

// On Android the real entry is `android_main` in the cdylib, invoked by
// NativeActivity. This stub exists only to satisfy cargo's binary requirement.
#[cfg(target_os = "android")]
fn main() {
    let _keep: usize = wasm_android_host::android_main as usize;
    std::hint::black_box(_keep);
}
