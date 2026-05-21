extern crate wasm_android_host;

#[cfg(not(target_os = "android"))]
fn main() {
    wasm_android_host::run();
}

// On Android there are two entry paths:
//   - NativeActivity APK  → `android_main` in the cdylib (winit event loop).
//   - `wart-host --standalone` → this bin, task-33 boot-model: a plain
//     privileged process that owns the display directly (no Activity).
// Without `--standalone` this bin is just a stub satisfying cargo's binary
// requirement; the APK never executes it.
#[cfg(target_os = "android")]
fn main() {
    if std::env::args().any(|a| a == "--standalone") {
        if let Err(e) = wasm_android_host::standalone::run() {
            eprintln!("wart-host --standalone: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    let _keep: usize = wasm_android_host::android_main as usize;
    std::hint::black_box(_keep);
}
