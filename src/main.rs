extern crate wasm_android_host;

#[cfg(not(target_os = "android"))]
fn main() {
    wasm_android_host::run();
}

// Android entry — four modes selected by argv:
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
//   `wart-host --run-once <app-id>`              → task-36 step-7 one-shot:
//                                                  load an installed
//                                                  wasi:cli/command app,
//                                                  call `wasi:cli/run.run()`
//                                                  once, exit with its
//                                                  status. Used for
//                                                  CLI/smoke consumers.
//   `wart-host --probe-ime`                      → task-40 session-2 probe:
//                                                  one-shot read-only call
//                                                  to IMMS
//                                                  (isImeTraceEnabled) to
//                                                  verify rsbinder reaches
//                                                  the input method service.
//   `wart-host --probe-ime-addclient`            → task-40 session-3 probe:
//                                                  stand up Bn-side servers
//                                                  for IInputMethodClient +
//                                                  IRemoteInputConnection,
//                                                  call addClient on IMMS,
//                                                  log the outcome (accept
//                                                  vs permission/identity
//                                                  rejection).
#[cfg(target_os = "android")]
fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--probe-ime") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::ime_impl::probe();
        return;
    }

    if args.iter().any(|a| a == "--probe-ime-addclient") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::ime_impl::probe_addclient();
        return;
    }

    if args.iter().any(|a| a == "--probe-ime-startinput") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::ime_impl::probe_startinput();
        return;
    }

    if args.iter().any(|a| a == "--probe-ime-showsoft") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::ime_impl::probe_showsoftinput();
        return;
    }

    if args.iter().any(|a| a == "--probe-wms-opensession") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::wms_impl::probe_wms_opensession();
        return;
    }

    // Task 45 step 1 — zygote server mode. Long-lived parent process that
    // preloads wasmtime::Engine and fork()s on each LAUNCH command from
    // /data/local/tmp/wart-zygote.sock. See tasks/45-wart-zygote-spike.md.
    if args.iter().any(|a| a == "--zygote") {
        // Optional preload-hint app-id; documentary at MVP (only engine
        // is preloaded today). `--zygote-preload <app-id>` keeps the CLI
        // shape forward-compatible with per-app Component preload later.
        let preload_hint = args.iter()
            .position(|a| a == "--zygote-preload")
            .and_then(|i| args.get(i + 1))
            .map(|s| s.as_str());
        if let Err(e) = wasm_android_host::zygote::serve(preload_hint) {
            eprintln!("wart-host --zygote: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    // Task 45 step 1 — zygote client mode. Connect to the zygote, write
    // LAUNCH <app-id>, print the child pid (or the structured ERR).
    if let Some(i) = args.iter().position(|a| a == "--zygote-launch") {
        let Some(app_id) = args.get(i + 1) else {
            eprintln!("wart-host --zygote-launch: requires <app-id>");
            std::process::exit(2);
        };
        match wasm_android_host::zygote::launch_client(app_id) {
            Ok(_pid) => return,
            Err(e) => {
                eprintln!("wart-host --zygote-launch: {e:#}");
                std::process::exit(1);
            }
        }
    }

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

    if let Some(i) = args.iter().position(|a| a == "--run-once") {
        let Some(app_id) = args.get(i + 1) else {
            eprintln!("wart-host --run-once: requires <app-id>");
            std::process::exit(2);
        };
        if let Err(e) = wasm_android_host::run_once::run(app_id) {
            eprintln!("wart-host --run-once: {e:#}");
            std::process::exit(1);
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
