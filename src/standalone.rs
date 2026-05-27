//! Standalone (no-`NativeActivity`) launch mode — task 33 boot-model.
//!
//! Reached via `wart-host --standalone`. The runtime runs as a plain
//! privileged process: it acquires a fullscreen surface from SurfaceFlinger
//! through the `libsf_surface` shim (no Activity, no winit `EventLoop`),
//! brings up EGL/Skia on it, and runs the WASM/Compose render loop.
//!
//! The render loop mirrors `lib.rs`'s `WindowEvent::RedrawRequested` handler
//! and the cold-start in `App::resumed`, minus winit. If no cwasm is present
//! it falls back to drawing the renderer test frame.

use std::path::Path;

use anyhow::Result;
use wasmtime::component::ResourceTable;
use wasmtime::{Engine, Store};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder};

use crate::app_loader::{self, AppLoader, AppRef, LoadedApp};
use crate::bindings;
use crate::{App, HostState};

/// Where the `libsf_surface` shim is deployed on the device.
const SHIM_SO: &str = "/data/local/tmp/libsf_surface.so";
/// Where the deployable AOT component is deployed on the device.
const CWASM_PATH: &str = "/data/local/tmp/skiko-component.cwasm";

pub fn run(app_id: Option<&str>) -> Result<()> {
    let engine = App::make_engine();
    run_with_engine(&engine, app_id)
}

/// Same as `run` but uses a caller-supplied engine. The task-45 zygote
/// child path (`LAUNCH_GUI <app-id>`) goes through here so the wasmtime
/// `Engine` allocated by the parent before `fork()` is reused (COW-
/// shared with siblings), instead of each child re-allocating a fresh
/// one — see [[project-app-lifecycle-and-packaging]] (Hybrid zygote
/// architecture lock).
pub fn run_with_engine(engine: &Engine, app_id: Option<&str>) -> Result<()> {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
    );
    // Surface guest WASI stderr + host panics to logcat (same as android_main).
    crate::wasi_stderr::redirect_stderr_to_logcat();
    log::info!("standalone: starting — no NativeActivity");

    // Task 33 Step 5 — clean-shutdown signals, crash marker, screen-state.
    crate::lifecycle_standalone::install_signal_handlers();
    crate::lifecycle_standalone::install_panic_hook();
    crate::lifecycle_standalone::drain_prior_crash_marker();

    // Task 46 step 4 — arbiter-driven foreground/background role.
    // Default is Foreground; SIGUSR1 demotes, SIGUSR2 promotes.
    crate::app_role::install_signal_handlers();

    // The shim's SurfaceComposerClient talks to SurfaceFlinger over binder.
    if let Err(e) = crate::binder::init() {
        log::warn!("standalone: binder init: {e}");
    }

    let sf = crate::sf_surface::SfSurface::create(SHIM_SO)?;
    log::info!(
        "standalone: surface {}x{} transform 0x{:x} (ANativeWindow={:p})",
        sf.width, sf.height, sf.transform, sf.native_window,
    );

    // The producer transform hint is only valid once EGL connects, so the
    // renderer queries it through this closure mid-`from_native_window`.
    let renderer = crate::canvas_impl::SkiaRenderer::from_native_window(
        sf.native_window, sf.width as u32, sf.height as u32,
        || sf.query_transform_hint(),
    )?;
    log::info!(
        "standalone: renderer up — EGL/Skia on the SurfaceFlinger window ({}x{})",
        renderer.width, renderer.height,
    );

    let loader = app_loader::default_for_target();
    let app_ref = match app_id {
        Some(id) => AppRef::Installed { app_id: id, version: None },
        None => AppRef::DevCwasm { candidates: &[Path::new(CWASM_PATH)] },
    };
    let result = match loader.load(engine, app_ref) {
        Ok(loaded) => {
            log::info!("standalone: loaded {}", loaded.source_label);
            run_cwasm_loop(engine, loaded, renderer, sf)
        }
        Err(e) => {
            log::warn!(
                "standalone: load failed ({e:#}) — falling back to test-frame loop"
            );
            run_test_loop(renderer)
        }
    };

    if result.is_ok() {
        crate::lifecycle_standalone::record_clean_exit();
    }
    result
}

/// The real render loop: instantiate the component and drive `render_frame`.
fn run_cwasm_loop(
    engine: &wasmtime::Engine,
    loaded: LoadedApp,
    renderer: crate::canvas_impl::SkiaRenderer,
    sf: crate::sf_surface::SfSurface,
) -> Result<()> {
    use bindings::my::skiko_gfx::lifecycle::State;

    // Logical surface size the guest must lay its UI out to. The winit path
    // gets this from a `WindowEvent::Resized`; standalone has no winit, so we
    // drive the guest's `on-resize` export explicitly once, below.
    let (logical_w, logical_h) = (renderer.logical_width, renderer.logical_height);

    // ── Cold start — mirrors App::resumed's cold path ────────────────────
    let mut wasi_builder = WasiCtxBuilder::new();
    wasi_builder.inherit_stdin().inherit_stdout();
    wasi_builder.stderr(crate::wasi_stderr::LogcatStderr);
    // Task 38 — installed apps with an `assets/` dir get it preopened
    // read-only at `/assets` in the guest. Dev paths skip this (no
    // install dir exists). Failure to preopen is non-fatal — log + run
    // without filesystem; guest reads will return ENOENT.
    if let Some(assets) = loaded.assets_dir() {
        match wasi_builder.preopened_dir(&assets, "/assets", DirPerms::READ, FilePerms::READ) {
            Ok(_)  => log::info!("standalone: preopened {} → /assets (read-only)", assets.display()),
            Err(e) => log::warn!("standalone: preopen {} failed: {e:#}", assets.display()),
        }
    }
    // Task 41 — /system/fonts/ preopen for the system-fonts dep.
    // Always-on, read-only. Guests that don't need fonts pay nothing
    // (just an unused preopen entry).
    match wasi_builder.preopened_dir("/system/fonts", "/system-fonts", DirPerms::READ, FilePerms::READ) {
        Ok(_)  => log::info!("standalone: preopened /system/fonts → /system-fonts (read-only)"),
        Err(e) => log::warn!("standalone: preopen /system/fonts failed: {e:#}"),
    }
    let wasi = wasi_builder.build();

    let host = HostState {
        renderer,
        scheduler: crate::scheduler_impl::SchedulerState::default(),
        lifecycle: crate::lifecycle_impl::LifecycleState {
            current: State::Resumed,
            pending: Some(State::Resumed),
        },
        clipboard: None,
        wasi,
        table: ResourceTable::new(),
        assets_dir: loaded.assets_dir(),
        #[cfg(feature = "profile")]
        growth_log: crate::profiling::GrowthLog::new(),
        #[cfg(feature = "profile")]
        frame_snapshot: crate::profiling::FrameSnapshotState::new(),
    };
    let mut store = Store::new(engine, host);
    #[cfg(feature = "profile")]
    {
        store.limiter(|h| &mut h.growth_log);
        store.call_hook(|_cx, kind| {
            crate::profiling::on_call_hook(kind);
            Ok(())
        });
    }

    let skiko = loaded.instantiate(&mut store)?;
    log::info!("standalone: component instantiated — entering render loop");

    // Tell the guest the surface size before the first frame, so Compose lays
    // out to the full panel (no winit `Resized` event to do this for us).
    if let Err(e) = skiko
        .my_skiko_gfx_renderer()
        .call_on_resize(&mut store, logical_w, logical_h)
    {
        log::warn!("standalone: on_resize({logical_w}x{logical_h}) failed: {e:#}");
    }

    // Screen on/off → Paused / Resumed. None if the device doesn't surface
    // the sysprop; the loop then just stays Resumed forever.
    let screen_rx = crate::lifecycle_standalone::spawn_screen_state_watcher();

    // Re-request input focus periodically — activity-backed windows AMS
    // resumes (launcher, last app) steal focus despite wart owning the
    // z-top SurfaceFlinger layer. Refresh roughly once per second.
    let focus_refresh_interval: u64 = 60; // frames @ ~60fps target

    // Task 46 step 4 — track previous arbiter-driven role so we react
    // exactly once per transition (not every frame). Newly forked
    // children default to Foreground, so the first frame logs a
    // promote-to-fg and ensures z-order + lifecycle.
    let mut last_role: Option<crate::app_role::AppRole> = None;

    // Task 47 step 3a — per-host control socket for arbiter-pushed
    // events. The accept thread listens on
    // /data/local/tmp/wart-host-<pid>.sock; the queue drain below
    // dispatches each event into the guest in the render-loop
    // thread (where the Store lives — wasmtime Store is !Send).
    match crate::ime_inbound::spawn_listener() {
        Ok(path) => log::info!("standalone: ime-inbound listening on {path}"),
        Err(e)   => log::warn!("standalone: ime-inbound spawn failed: {e:#}"),
    }

    // ── Render loop — mirrors WindowEvent::RedrawRequested, no winit ─────
    let frame_target = std::time::Duration::from_millis(16);
    let mut frame: u64 = 0;
    loop {
        // Step 5 — SIGTERM / SIGINT / SIGHUP from launcher trap or operator.
        if crate::lifecycle_standalone::should_shutdown() {
            log::info!("standalone: shutdown signal — exiting render loop");
            break;
        }

        // Task 46 step 4 — arbiter role transition. SIGUSR1/SIGUSR2
        // updates an atomic; we observe it once per frame. On change:
        //   Foreground → Background: SF set_layer(0), set_visible(false),
        //                            lifecycle Paused.
        //   Background → Foreground: SF set_layer(MAX), set_visible(true),
        //                            request_focus, lifecycle Resumed.
        // Children unaware of the new role (older shim, no signals
        // received) stay Foreground — same behavior as pre-step-4.
        let cur_role = crate::app_role::role();
        if last_role != Some(cur_role) {
            use crate::app_role::AppRole;
            log::info!("standalone: role transition {last_role:?} → {cur_role:?}");
            match cur_role {
                AppRole::Foreground => {
                    sf.set_layer(i32::MAX);
                    sf.set_visible(true);
                    sf.request_focus();
                    let target = bindings::my::skiko_gfx::lifecycle::State::Resumed;
                    if store.data().lifecycle.current != target {
                        store.data_mut().lifecycle.current = target;
                        if let Err(e) = skiko
                            .my_skiko_gfx_renderer()
                            .call_on_lifecycle_changed(&mut store, target as u32)
                        {
                            log::warn!("standalone: on_lifecycle_changed(fg→Resumed) failed: {e:#}");
                        }
                    }
                }
                AppRole::Background => {
                    sf.set_layer(0);
                    sf.set_visible(false);
                    let target = bindings::my::skiko_gfx::lifecycle::State::Paused;
                    if store.data().lifecycle.current != target {
                        store.data_mut().lifecycle.current = target;
                        if let Err(e) = skiko
                            .my_skiko_gfx_renderer()
                            .call_on_lifecycle_changed(&mut store, target as u32)
                        {
                            log::warn!("standalone: on_lifecycle_changed(bg→Paused) failed: {e:#}");
                        }
                    }
                }
            }
            last_role = Some(cur_role);
        }

        // Drain any screen-state transitions accumulated since last frame.
        // Last one wins — we only care about the final state per frame.
        if let Some(rx) = screen_rx.as_ref() {
            let mut latest = None;
            while let Ok(s) = rx.try_recv() { latest = Some(s); }
            if let Some(s) = latest {
                let target = if s.is_live() {
                    bindings::my::skiko_gfx::lifecycle::State::Resumed
                } else {
                    bindings::my::skiko_gfx::lifecycle::State::Paused
                };
                if store.data().lifecycle.current != target {
                    store.data_mut().lifecycle.current = target;
                    if let Err(e) = skiko
                        .my_skiko_gfx_renderer()
                        .call_on_lifecycle_changed(&mut store, target as u32)
                    {
                        log::warn!(
                            "standalone: on_lifecycle_changed({target:?}) failed: {e:#}"
                        );
                    }
                }
            }
        }

        let t0 = std::time::Instant::now();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        // Drain InputFlinger events and dispatch them to the guest. The
        // shim's input channel is non-blocking; this is the standalone
        // equivalent of winit's touch/key events (task 33 Step 3).
        let mut input_buf = [crate::sf_surface::SfInputEvent::default(); 32];
        for ev in sf.poll_input(&mut input_buf) {
            if ev.kind <= 3 {
                if let Err(e) = crate::input::dispatch_pointer_v2(
                    &skiko, &mut store, ev.kind as u8,
                    ev.pointer_id as u32, ev.x, ev.y, ev.pressure,
                ) {
                    log::warn!("standalone: dispatch_pointer_v2 failed: {e:#}");
                }
            } else if ev.kind == 10 || ev.kind == 11 {
                // 10=key-down, 11=key-up. Action byte (0/1) matches the
                // dispatch_*_v1/v2 contract.
                let action = if ev.kind == 10 { 0u8 } else { 1u8 };
                if let Err(e) = crate::input::dispatch_android_key(
                    &skiko, &mut store, action, ev.key_code, ev.meta_state,
                ) {
                    log::warn!("standalone: dispatch_android_key failed: {e:#}");
                }
            }
        }

        // Task 47 step 3a — drain arbiter-pushed IME events (key
        // synthesis from a virtual keyboard). Same per-frame
        // pattern as the InputFlinger drain above.
        for ev in crate::ime_inbound::drain_queue() {
            match ev {
                crate::ime_inbound::InboundEvent::KeyEvent { code_point, key_id, action } => {
                    if let Err(e) = crate::input::dispatch_key_v2(
                        &skiko, &mut store, action, code_point, key_id,
                    ) {
                        log::warn!("standalone: dispatch_key_v2 (ime-inbound) failed: {e:#}");
                    }
                }
            }
        }

        // Drain scheduler callbacks whose deadline has passed.
        let due = store.data_mut().scheduler.drain_due(std::time::Instant::now());
        for cb in due {
            if let Err(e) = skiko
                .my_skiko_gfx_renderer()
                .call_on_scheduled_callback(&mut store, cb)
            {
                log::warn!("standalone: on_scheduled_callback({cb}) failed: {e:#}");
            }
        }

        let result = skiko
            .my_skiko_gfx_renderer()
            .call_render_frame(&mut store, nanos);

        // Fire the pending lifecycle transition after the first successful
        // frame (gives appMain a chance to register its observer first).
        if result.is_ok() {
            if let Some(state) = store.data_mut().lifecycle.pending.take() {
                if let Err(e) = skiko
                    .my_skiko_gfx_renderer()
                    .call_on_lifecycle_changed(&mut store, state as u32)
                {
                    log::warn!("standalone: on_lifecycle_changed failed: {e:#}");
                }
            }
        }

        if let Err(e) = result {
            let msg = format!("{e:?}");
            if msg.contains("cannot enter component instance") {
                log::error!("standalone: component instance poisoned — exiting");
                return Err(anyhow::anyhow!("render_frame fatal: {msg}"));
            }
            log::error!("standalone: render_frame #{frame} error: {e:#}");
        }

        frame += 1;
        if frame <= 3 || frame % 600 == 0 {
            log::info!("standalone: rendered frame {frame}");
        }
        // Task 46 step 4 — only the foreground app fights for focus.
        // Background apps that keep stealing focus would defeat the
        // arbiter's policy + spam the launcher. Frequency unchanged
        // (~1 s) when foreground.
        if frame % focus_refresh_interval == 0 && crate::app_role::is_foreground() {
            sf.request_focus();
        }

        let elapsed = t0.elapsed();
        if elapsed < frame_target {
            std::thread::sleep(frame_target - elapsed);
        }
    }

    // ── Shutdown — fire Destroyed so the LifecycleRegistry walks
    //              Resumed → Paused → Stopped → Created → Destroyed, giving
    //              Compose observers a chance to flush state. Drain a few
    //              frames after so the resulting recompositions render
    //              before EGL/binder teardown via the SfSurface Drop chain.
    log::info!("standalone: dispatching Destroyed → drain frames → exit");
    let final_state = bindings::my::skiko_gfx::lifecycle::State::Destroyed;
    store.data_mut().lifecycle.current = final_state;
    if let Err(e) = skiko
        .my_skiko_gfx_renderer()
        .call_on_lifecycle_changed(&mut store, final_state as u32)
    {
        log::warn!("standalone: on_lifecycle_changed(Destroyed) failed: {e:#}");
    }
    let drain_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    for _ in 0..3 {
        if let Err(e) = skiko
            .my_skiko_gfx_renderer()
            .call_render_frame(&mut store, drain_nanos)
        {
            log::warn!("standalone: drain render_frame failed: {e:#}");
            break;
        }
    }
    log::info!("standalone: clean exit");
    Ok(())
}

/// Fallback when no cwasm is deployed — draws the built-in test frame.
fn run_test_loop(mut renderer: crate::canvas_impl::SkiaRenderer) -> Result<()> {
    log::info!("standalone: test-frame loop (no cwasm)");
    let mut frame: u64 = 0;
    loop {
        if crate::lifecycle_standalone::should_shutdown() {
            log::info!("standalone: shutdown signal — exiting test loop");
            return Ok(());
        }
        renderer.draw_test_frame();
        frame += 1;
        if frame <= 3 || frame % 300 == 0 {
            log::info!("standalone: test frame {frame}");
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}
