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

    // Audio mic-capture de-risk (does openStream(INPUT) succeed for our caller?).
    if args.iter().any(|a| a == "--probe-audio-capture") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::audio_impl::probe_capture();
        return;
    }

    // Task 76 P1 — call-order full-duplex capture probe: --probe-audio-duplex <preset>.
    if let Some(i) = args.iter().position(|a| a == "--probe-audio-duplex") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        let preset = args.get(i + 1).and_then(|s| s.parse::<i32>().ok()).unwrap_or(6);
        wasm_android_host::audio_impl::probe_duplex(preset);
        return;
    }

    // Audio mic→speaker loopback (full capture path: hear yourself).
    if args.iter().any(|a| a == "--probe-audio-loopback") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::audio_impl::probe_loopback();
        return;
    }

    // Task-76 audio capability probe (read-only): dump the device's real audio
    // picture (ports/routing/volumes via dumpsys + binder reachability) and a
    // typed device model. See audio_caps.rs.
    if args.iter().any(|a| a == "--probe-audio-caps") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::audio_caps::probe();
        return;
    }

    // Task-93 video spike: open the camera + HW VP8-encode its frames under
    // --no-art, report fps / first-frame latency. `wart-host --probe-video`.
    if args.iter().any(|a| a == "--probe-video") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::video_probe::probe_video();
        return;
    }

    // Task-76 P8 volume probe: read media volume range + speaker/earpiece index,
    // set speaker to max, read back, restore. Proves the write path.
    if args.iter().any(|a| a == "--probe-audio-volume") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::audio_policy_impl::probe_volume();
        return;
    }

    // Standalone tone player (same media.aaudio MMAP path the host uses) — for
    // A/B testing audio routing with vs without system_server. `--play-tone [ms]
    // [hz] [vol]`. Defaults 8000ms, 440Hz, 0.6.
    if let Some(i) = args.iter().position(|a| a == "--play-tone") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        let ms  = args.get(i + 1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(8000);
        let hz  = args.get(i + 2).and_then(|s| s.parse::<f32>().ok()).unwrap_or(440.0);
        let vol = args.get(i + 3).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.6);
        wasm_android_host::audio_impl::play_tone(ms, hz, vol);
        return;
    }

    // Task 97 bug #1 — reproduce the SHARED-output suspend stall on the call path
    // (and A/B the silence-pump fix). `--probe-call-stall [secs] [speaker0|1]
    // [drain0|1] [pump0|1]`. Defaults: 8s resume window, earpiece (speaker=0),
    // drain-during-underflow OFF, pump OFF (the permanent-stall baseline). pump=1
    // opens via the guest create_track path (spawns the silence-pump fix) → the
    // same underflow should NOT stall. Drive logcat for "Suspending stream".
    if let Some(i) = args.iter().position(|a| a == "--probe-call-stall") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        let secs    = args.get(i + 1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(8);
        let speaker = args.get(i + 2).map(|s| s == "1").unwrap_or(false);
        let drain   = args.get(i + 3).map(|s| s == "1").unwrap_or(false);
        let pump    = args.get(i + 4).map(|s| s == "1").unwrap_or(false);
        wasm_android_host::audio_impl::probe_call_stall(secs, speaker, drain, pump);
        return;
    }

    // Task 97 bug #5 — verify the earpiece/speaker toggle re-routes the shared
    // MEDIA output (setDevicesRoleForStrategy) instead of pinning a per-stream
    // deviceId (which -889s). `--probe-route-toggle`.
    if args.iter().any(|a| a == "--probe-route-toggle") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::audio_impl::probe_route_toggle();
        return;
    }

    // Task 98 — native AudioFlinger backend smoke test, run from *inside* wart-host
    // (which already hosts a binder thread-pool, unlike the standalone af_probe).
    // Opens an AudioTrack directly via the `audioclient` crate (IAudioFlingerService
    // .createTrack → cblk ring) and writes a 440 Hz tone. `--probe-audioclient [secs]
    // [hz] [vol]`. This is the on-device validation that the AudioFlinger-direct path
    // makes sound without AAudioService.
    if let Some(i) = args.iter().position(|a| a == "--probe-audioclient") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        let secs = args.get(i + 1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(3);
        let hz   = args.get(i + 2).and_then(|s| s.parse::<f32>().ok()).unwrap_or(440.0);
        let vol  = args.get(i + 3).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.3);
        probe_audioclient(secs, hz, vol);
        return;
    }

    // Task 98 — createTrack request-variant matrix: isolate which CreateTrackRequest
    // field audioserver silently rejects with BAD_VALUE (it logs nothing server-side,
    // even at VERBOSE, in both ART and --no-art). `--probe-audioclient-matrix`.
    if args.iter().any(|a| a == "--probe-audioclient-matrix") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        audioclient::probe_matrix();
        return;
    }

    // --no-art audio bring-up: replicate AudioService's boot volume/device init
    // (initStreamVolume + setStreamVolumeIndex + mode/force-use) so audio is
    // audible without system_server. Run by run-hybrid-stack after audioserver.
    if args.iter().any(|a| a == "--init-audio-policy") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::audio_policy_impl::init_audio_policy();
        return;
    }

    // Task-76 #6 — enumerate audio ports over binder (listAudioPorts) instead
    // of dumpsys; tests AudioPortFw decode at runtime.
    if args.iter().any(|a| a == "--probe-audio-ports") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::audio_policy_impl::probe_list_audio_ports();
        return;
    }

    // Task-76 routing core (step 4): build the live device model and log the
    // resolved stream plan for every intent (read-only). See audio_routing.rs.
    if args.iter().any(|a| a == "--probe-audio-route") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::audio_routing::probe_routes();
        return;
    }

    // Task-76 audio state matrix (step 3): targeted self-restoring on-device
    // opens filling the (usage × mode × device × sharing × format × channels)
    // matrix. Restores phone state to NORMAL after the comms-mode cells.
    if args.iter().any(|a| a == "--probe-audio-matrix") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::audio_caps::probe_matrix();
        return;
    }

    // Call-audio reachability (read-only): does a root caller reach
    // media.audio_policy? Logs phone state + COMMUNICATION routing.
    if args.iter().any(|a| a == "--probe-audio-policy") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        wasm_android_host::audio_policy_impl::probe();
        return;
    }

    // Call-audio routing WRITE probe: setForceUse(COMMUNICATION, speaker|earpiece)
    // then restore. `--probe-audio-policy-route speaker` | `... earpiece`.
    if let Some(i) = args.iter().position(|a| a == "--probe-audio-policy-route") {
        android_logger::init_once(
            android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
        );
        let speaker = args.get(i + 1).map(|s| s == "speaker").unwrap_or(false);
        wasm_android_host::audio_policy_impl::probe_route(speaker);
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
    // LAUNCH <app-id> (headless / wasi:cli/command), print the child pid
    // (or the structured ERR).
    if let Some(i) = args.iter().position(|a| a == "--zygote-launch") {
        let Some(app_id) = args.get(i + 1) else {
            eprintln!("wart-host --zygote-launch: requires <app-id>");
            std::process::exit(2);
        };
        match wasm_android_host::zygote::launch_client(app_id, /*gui=*/ false, /*overlay=*/ false) {
            Ok(_pid) => return,
            Err(e) => {
                eprintln!("wart-host --zygote-launch: {e:#}");
                std::process::exit(1);
            }
        }
    }

    // Task 45 step 2 — same as above but for full Compose render loop.
    // Forks via the zygote, child runs standalone::run_with_engine
    // against the preloaded engine.
    // Accepts an optional <app-id>; if omitted, the child falls back
    // to the dev cwasm at /data/local/tmp/skiko-component.cwasm.
    // `--overlay` (task 47 step 3c) acquires a bottom-strip overlay
    // surface in the child instead of a fullscreen one — used for
    // IME apps such as `war.ime.keyboard`.
    if let Some(i) = args.iter().position(|a| a == "--zygote-launch-gui") {
        let app_id = args.get(i + 1).map(|s| s.as_str()).unwrap_or("");
        let overlay = args.iter().any(|a| a == "--overlay");
        match wasm_android_host::zygote::launch_client(app_id, /*gui=*/ true, overlay) {
            Ok(_pid) => return,
            Err(e) => {
                eprintln!("wart-host --zygote-launch-gui: {e:#}");
                std::process::exit(1);
            }
        }
    }

    // Task 46 step 1 — graceful + forceful KILL of a child via the
    // zygote socket. Validates server-side that the pid is one of the
    // zygote's own children before signaling.
    for (flag, force) in [
        ("--zygote-kill", false),
        ("--zygote-kill-force", true),
    ] {
        if let Some(i) = args.iter().position(|a| a == flag) {
            let Some(pid_s) = args.get(i + 1) else {
                eprintln!("wart-host {flag}: requires <pid>");
                std::process::exit(2);
            };
            let Ok(pid) = pid_s.parse::<i32>() else {
                eprintln!("wart-host {flag}: <pid> must be an integer");
                std::process::exit(2);
            };
            match wasm_android_host::zygote::kill_client(pid, force) {
                Ok(()) => return,
                Err(e) => {
                    eprintln!("wart-host {flag}: {e:#}");
                    std::process::exit(1);
                }
            }
        }
    }

    // Task 46 step 2 — PRELOAD socket command client. Used by the
    // installer (after upgrades) and by the future wart-arbiter
    // (predictive warm-up before launches). System bundles are
    // auto-preloaded at zygote startup; this command handles user
    // apps and post-upgrade refreshes.
    if let Some(i) = args.iter().position(|a| a == "--zygote-preload") {
        let Some(app_id) = args.get(i + 1) else {
            eprintln!("wart-host --zygote-preload: requires <app-id>");
            std::process::exit(2);
        };
        match wasm_android_host::zygote::preload_client(app_id) {
            Ok(()) => return,
            Err(e) => {
                eprintln!("wart-host --zygote-preload: {e:#}");
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

    if args.iter().any(|a| a.starts_with("--standalone")) {
        let app_id = args.iter()
            .position(|a| a == "--app")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str);
        // Overlay mode: `--standalone-overlay` = bottom strip (IME,
        // task 47); `--standalone-overlay-bottom-bar` = thin bottom nav
        // strip (taskbar, task 56); `--standalone-overlay-top` = top
        // strip (status bar, task 55); none = fullscreen.
        use wasm_android_host::standalone::OverlayMode;
        let mode = if args.iter().any(|a| a == "--standalone-overlay-top") {
            OverlayMode::Top
        } else if args.iter().any(|a| a == "--standalone-overlay-bottom-bar") {
            OverlayMode::BottomBar
        } else if args.iter().any(|a| a == "--standalone-overlay-lock") {
            OverlayMode::Lock
        } else if args.iter().any(|a| a == "--standalone-overlay") {
            OverlayMode::Bottom
        } else {
            OverlayMode::None
        };
        if let Err(e) = wasm_android_host::standalone::run(app_id, mode) {
            eprintln!("wart-host --standalone: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    let _keep: usize = wasm_android_host::android_main as usize;
    std::hint::black_box(_keep);
}

// Task 98 — AudioFlinger-direct smoke test (see the `--probe-audioclient` flag).
// Runs inside wart-host so the local IAudioTrackCallback Bn marshals against a real
// binder thread-pool (the standalone af_probe couldn't — kernel rejected the
// oneway-spam ioctl with EINVAL). Opens an output AudioTrack via the `audioclient`
// crate, writes a `hz` sine at `vol` for `secs`, starting after the first accepted
// write, then closes.
#[cfg(target_os = "android")]
fn probe_audioclient(secs: u64, hz: f32, vol: f32) {
    eprintln!("probe-audioclient: open_output(MEDIA/MUSIC, 48k stereo)…");
    let h = audioclient::open_output(audioclient::OutputConfig {
        sample_rate: 48_000,
        channels: 2,
        usage: 1,        // AUDIO_USAGE_MEDIA
        content_type: 2, // AUDIO_CONTENT_TYPE_MUSIC
    });
    if h == 0 {
        eprintln!("probe-audioclient: open_output FAILED (see logcat tag 'audioclient')");
        std::process::exit(1);
    }
    eprintln!("probe-audioclient: handle={h} — writing {hz} Hz tone for {secs}s…");

    let sr = 48_000.0_f32;
    let mut phase = 0.0_f32;
    let mut started = false;
    let mut total = 0usize;
    let mut zero_ticks = 0u32;
    let t0 = std::time::Instant::now();
    while t0.elapsed().as_secs() < secs {
        let mut buf: Vec<f32> = Vec::with_capacity(480 * 2);
        for _ in 0..480 {
            let s = phase.sin() * vol;
            phase += 2.0 * std::f32::consts::PI * hz / sr;
            if phase > 2.0 * std::f32::consts::PI {
                phase -= 2.0 * std::f32::consts::PI;
            }
            buf.push(s);
            buf.push(s);
        }
        let n = audioclient::write(h, &buf);
        total += n;
        if n == 0 {
            zero_ticks += 1;
        }
        if !started && n > 0 {
            let ok = audioclient::start(h);
            started = true;
            eprintln!("probe-audioclient: started (IAudioTrack.start ok={ok})");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    eprintln!("probe-audioclient: wrote {total} frames (zero-ticks={zero_ticks}); closing");
    audioclient::stop(h);
    audioclient::close(h);
    eprintln!("probe-audioclient: done");
}
