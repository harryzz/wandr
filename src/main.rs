extern crate wasm_android_host;

#[cfg(not(target_os = "android"))]
fn main() {
    wasm_android_host::run();
}

// Android entry — three modes selected by argv:
//
//   `wart-host`                                  → NativeActivity stub (this
//                                                  bin is never executed by
//                                                  the APK; android_main in
//                                                  the cdylib is the entry).
//   `wart-host --install <warpkg-dir>`           → task-35 installer: read
//                                                  bundle, AOT-precompile,
//                                                  write `cache-key.toml`.
//   `wart-host --standalone [--app <app-id>]`    → task-33 boot-model:
//                                                  privileged process that
//                                                  owns the display. Loads
//                                                  the dev cwasm at
//                                                  /data/local/tmp by default;
//                                                  with `--app`, loads via
//                                                  AppRef::Installed.
#[cfg(target_os = "android")]
fn main() {
    let args: Vec<String> = std::env::args().collect();

    if let Some(i) = args.iter().position(|a| a == "--install") {
        let Some(warpkg) = args.get(i + 1) else {
            eprintln!("wart-host --install: requires a <warpkg-dir> path");
            std::process::exit(2);
        };
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        match wasm_android_host::install_warpkg(std::path::Path::new(warpkg)) {
            Ok(installed) => {
                println!(
                    "installed: {} v{} → {}",
                    installed.app_id, installed.version, installed.install_dir.display(),
                );
            }
            Err(e) => {
                eprintln!("wart-host --install: {e:#}");
                std::process::exit(1);
            }
        }
        return;
    }

    if args.iter().any(|a| a == "--standalone") {
        let app_id = args.iter()
            .position(|a| a == "--app")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str);
        if let Err(e) = wasm_android_host::standalone::run(app_id) {
            eprintln!("wart-host --standalone: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    let _keep: usize = wasm_android_host::android_main as usize;
    std::hint::black_box(_keep);
}
