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

use anyhow::Result;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::Store;
use wasmtime_wasi::WasiCtxBuilder;

use crate::bindings;
use crate::{App, HostState};

/// Where the `libsf_surface` shim is deployed on the device.
const SHIM_SO: &str = "/data/local/tmp/libsf_surface.so";
/// Where the deployable AOT component is deployed on the device.
const CWASM_PATH: &str = "/data/local/tmp/skiko-component.cwasm";

pub fn run() -> Result<()> {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
    );
    // Surface guest WASI stderr + host panics to logcat (same as android_main).
    crate::wasi_stderr::redirect_stderr_to_logcat();
    log::info!("standalone: starting — no NativeActivity");

    // The shim's SurfaceComposerClient talks to SurfaceFlinger over binder.
    if let Err(e) = crate::binder::init() {
        log::warn!("standalone: binder init: {e}");
    }

    let sf = crate::sf_surface::SfSurface::create(SHIM_SO)?;
    log::info!(
        "standalone: surface {}x{} (ANativeWindow={:p})",
        sf.width, sf.height, sf.native_window,
    );

    let renderer = crate::canvas_impl::SkiaRenderer::from_native_window(
        sf.native_window, sf.width as u32, sf.height as u32,
    )?;
    log::info!(
        "standalone: renderer up — EGL/Skia on the SurfaceFlinger window ({}x{})",
        renderer.width, renderer.height,
    );

    // Same Engine config as the NativeActivity path — the AOT cwasm contract
    // depends on it (gc / function-references / exceptions / stack sizes).
    let engine = App::make_engine();
    match unsafe { Component::deserialize_file(&engine, CWASM_PATH) } {
        Ok(component) => {
            log::info!("standalone: loaded cwasm {CWASM_PATH}");
            run_cwasm_loop(engine, component, renderer)
        }
        Err(e) => {
            log::warn!(
                "standalone: no cwasm at {CWASM_PATH} ({e}) — falling back to \
                 test-frame loop"
            );
            run_test_loop(renderer)
        }
    }
}

/// The real render loop: instantiate the component and drive `render_frame`.
fn run_cwasm_loop(
    engine: wasmtime::Engine,
    component: Component,
    renderer: crate::canvas_impl::SkiaRenderer,
) -> Result<()> {
    use bindings::my::skiko_gfx::lifecycle::State;

    // ── Cold start — mirrors App::resumed's cold path ────────────────────
    let mut wasi_builder = WasiCtxBuilder::new();
    wasi_builder.inherit_stdin().inherit_stdout();
    wasi_builder.stderr(crate::wasi_stderr::LogcatStderr);
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
        #[cfg(feature = "profile")]
        growth_log: crate::profiling::GrowthLog::new(),
        #[cfg(feature = "profile")]
        frame_snapshot: crate::profiling::FrameSnapshotState::new(),
    };
    let mut store = Store::new(&engine, host);
    #[cfg(feature = "profile")]
    {
        store.limiter(|h| &mut h.growth_log);
        store.call_hook(|_cx, kind| {
            crate::profiling::on_call_hook(kind);
            Ok(())
        });
    }

    let mut linker: Linker<HostState> = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    bindings::SkikoUi::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |s| s)?;

    let skiko = bindings::SkikoUi::instantiate(&mut store, &component, &linker)?;
    log::info!("standalone: component instantiated — entering render loop");

    // ── Render loop — mirrors WindowEvent::RedrawRequested, no winit ─────
    let frame_target = std::time::Duration::from_millis(16);
    let mut frame: u64 = 0;
    loop {
        let t0 = std::time::Instant::now();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

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

        let elapsed = t0.elapsed();
        if elapsed < frame_target {
            std::thread::sleep(frame_target - elapsed);
        }
    }
}

/// Fallback when no cwasm is deployed — draws the built-in test frame.
fn run_test_loop(mut renderer: crate::canvas_impl::SkiaRenderer) -> Result<()> {
    log::info!("standalone: test-frame loop (no cwasm)");
    let mut frame: u64 = 0;
    loop {
        renderer.draw_test_frame();
        frame += 1;
        if frame <= 3 || frame % 300 == 0 {
            log::info!("standalone: test frame {frame}");
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}
