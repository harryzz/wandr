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

/// Where + whether this standalone process takes an overlay strip vs a
/// fullscreen surface.
///   - `None`      → fullscreen app (launcher, regular apps).
///   - `Bottom`    → bottom-strip overlay, tall + resizable (IME keyboard, task 47).
///   - `BottomBar` → thin bottom nav strip, always-visible (taskbar, task 56).
///   - `Top`       → top-strip overlay (status bar, task 55).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OverlayMode {
    None,
    Bottom,
    BottomBar,
    Top,
}

pub fn run(app_id: Option<&str>, mode: OverlayMode) -> Result<()> {
    let engine = App::make_engine();
    run_with_engine(&engine, app_id, mode)
}

/// Initial bottom-overlay (IME) panel height in physical pixels. The IME
/// guest may resize via `my:skiko-gfx/keyboard.request-overlay-height`.
const INITIAL_OVERLAY_PX: i32 = 1200;

/// Taskbar (BottomBar) strip height in physical pixels — the Android-style
/// Back/Home/Recents nav bar (task 56). Env-tunable via `WART_TASKBAR_PX`.
pub fn taskbar_height_px() -> u32 {
    std::env::var("WART_TASKBAR_PX")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(150)
}


/// Same as `run` but uses a caller-supplied engine. The task-45 zygote
/// child path (`LAUNCH_GUI <app-id>`) goes through here so the wasmtime
/// `Engine` allocated by the parent before `fork()` is reused (COW-
/// shared with siblings), instead of each child re-allocating a fresh
/// one — see [[project-app-lifecycle-and-packaging]] (Hybrid zygote
/// architecture lock).
///
/// `overlay=true` (task 47 step 3c) requests a bottom-strip overlay
/// SurfaceControl from the shim. Falls back to fullscreen with a
/// logged warning if the shim doesn't export `sf_create_overlay_surface`
/// (e.g. an older `libsf_surface.so` predating step 3c).
pub fn run_with_engine(engine: &Engine, app_id: Option<&str>, mode: OverlayMode) -> Result<()> {
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

    let sf = match mode {
        OverlayMode::Bottom | OverlayMode::BottomBar | OverlayMode::Top => {
            // Geometry: full panel width (w=0). Top status bar at y=0,
            // height STATUS_BAR_PX; bottom IME anchored (y=-1), height
            // INITIAL_OVERLAY_PX. The runtime owns the semantics; the
            // shim is geometry-generic (per the surface-abstraction
            // discussion — task 55).
            let (y, h, label) = match mode {
                // Status-bar height is a single host parameter
                // (WART_STATUSBAR_PX); the status.bar-height() verb +
                // the launcher's inset derive from the same source.
                OverlayMode::Top => (0, crate::status_impl::status_bar_height_px() as i32, "top"),
                // Taskbar — bottom-anchored, thin fixed height (task 56).
                OverlayMode::BottomBar => (-1, taskbar_height_px() as i32, "bottom-bar"),
                _ => (-1, INITIAL_OVERLAY_PX, "bottom"),
            };
            match crate::sf_surface::SfSurface::create_overlay(SHIM_SO, 0, y, 0, h) {
                Ok(sf) => {
                    log::info!(
                        "standalone: {label} overlay surface {}x{} transform 0x{:x} \
                         (h={} px, ANativeWindow={:p})",
                        sf.width, sf.height, sf.transform, h, sf.native_window,
                    );
                    sf
                }
                Err(e) => {
                    log::warn!(
                        "standalone: overlay surface unavailable ({e:#}) — \
                         falling back to fullscreen. Rebuild libsf_surface.so on a-03."
                    );
                    crate::sf_surface::SfSurface::create(SHIM_SO)?
                }
            }
        }
        OverlayMode::None => {
            let sf = crate::sf_surface::SfSurface::create(SHIM_SO)?;
            log::info!(
                "standalone: surface {}x{} transform 0x{:x} (ANativeWindow={:p})",
                sf.width, sf.height, sf.transform, sf.native_window,
            );
            sf
        }
    };

    // The producer transform hint is only valid once EGL connects, so the
    // renderer queries it through this closure mid-`from_native_window`.
    let mut renderer = crate::canvas_impl::SkiaRenderer::from_native_window(
        sf.native_window, sf.width as u32, sf.height as u32,
        || sf.query_transform_hint(),
    )?;
    log::info!(
        "standalone: renderer up — EGL/Skia on the SurfaceFlinger window ({}x{})",
        renderer.width, renderer.height,
    );

    // Task 56 — chrome content insets for a fullscreen app: reserve the
    // status-bar (top) + taskbar (bottom) strips so the app never draws
    // under the chrome, in any orientation. Driven by env (the shell sets
    // WART_INSET_TOP/BOTTOM to the chrome heights when launching the
    // zygote); unset → 0 → the app gets the full native display size (a
    // true immersive fullscreen app with no chrome). Only fullscreen
    // (None) mode insets — the chrome overlays themselves render full.
    if mode == OverlayMode::None {
        let env_px = |k: &str| {
            std::env::var(k).ok().and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0)
        };
        let (top, bottom) = (env_px("WART_INSET_TOP"), env_px("WART_INSET_BOTTOM"));
        if top > 0 || bottom > 0 {
            renderer.set_insets(top, bottom);
        }
    }

    let loader = app_loader::default_for_target();
    let app_ref = match app_id {
        Some(id) => AppRef::Installed { app_id: id, version: None },
        None => AppRef::DevCwasm { candidates: &[Path::new(CWASM_PATH)] },
    };
    let result = match loader.load(engine, app_ref) {
        Ok(loaded) => {
            log::info!("standalone: loaded {}", loaded.source_label);
            // Z-stack (task 56): fullscreen apps at the bottom; the
            // taskbar nav strip above apps but below the IME/status-bar
            // chrome, so the keyboard (and status bar) draw over it; the
            // IME + status bar at the top.
            let fg_layer = match mode {
                OverlayMode::None      => 0x4000_0000,
                OverlayMode::BottomBar => 0x6000_0000,
                OverlayMode::Bottom | OverlayMode::Top => i32::MAX,
            };
            // Task 62/63 — rotation gate. Fullscreen apps rotate UNLESS they
            // explicitly declare `orientation = "locked"` (absent ⇒ rotates,
            // preserving task-43); overlays rotate only on explicit
            // `orientation = "auto"`. A locked fullscreen app additionally
            // publishes a system lock (below) so the chrome stays portrait.
            let is_locked = loaded.orientation_locked();
            let rotates = if mode == OverlayMode::None {
                !is_locked
            } else {
                loaded.rotation_policy()
            };
            run_cwasm_loop(engine, loaded, renderer, sf, mode, rotates, is_locked, fg_layer)
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

/// Map the Device Orientation HAL value to the renderer's dihedral
/// `orient` code (task 43).
///
/// The HAL reports the rotation the *display* should adopt as a
/// `Surface.ROTATION_*` index: 0=0°, 1=90° CCW, 2=180°, 3=90° CW. To keep
/// the UI upright we pre-rotate the *content* by the inverse, which in
/// the renderer's bitmask is: 0→0 (identity), 1→4 (ROT_90 CW), 2→3
/// (ROT_180), 3→7 (ROT_270 CCW).
///
/// If a given panel turns out to rotate the wrong way (content tilts
/// opposite the device), swap the `1 => 4` and `3 => 7` arms — that's the
/// only handedness assumption here, and it's panel-specific.
fn device_rotation_to_orient(rot: u32) -> u32 {
    match rot & 3 {
        0 => 0, // portrait — identity
        1 => 4, // ROT_90
        2 => 3, // ROT_180
        _ => 7, // ROT_270
    }
}

/// Task 63 — global system orientation lock (cross-process IPC). The
/// foreground fullscreen app writes `1` here when it declares
/// `orientation = "locked"`, `0` otherwise; the system chrome overlays
/// (status bar / taskbar / IME) read it each frame and stay portrait while
/// it's set, regardless of the device sensor. A plain file (alongside the
/// zygote/arbiter sockets) — the status bar / taskbar are launched directly,
/// not arbiter-tracked, so a per-socket push can't reach them; a global file
/// they all poll is the simplest reliable channel.
const ORIENT_LOCK_PATH: &str = "/data/local/tmp/wart-orient-lock";

/// Publish the system orientation lock (foreground fullscreen app only).
fn publish_orientation_lock(locked: bool) {
    if let Err(e) = std::fs::write(ORIENT_LOCK_PATH, if locked { "1" } else { "0" }) {
        log::warn!("standalone: could not publish orientation lock: {e:#}");
    }
}

/// Read the system orientation lock. Absent / unreadable ⇒ unlocked.
fn orientation_lock_active() -> bool {
    std::fs::read_to_string(ORIENT_LOCK_PATH)
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

/// Physical compass edge of the portrait panel buffer.
#[derive(Clone, Copy)]
enum Edge { North, South, East, West }

/// Task 62 — the panel-buffer-space rect `(x, y, w, h)` an anchored chrome
/// overlay must occupy for a given content-rotation `orient` (0/4/3/7 from
/// [`device_rotation_to_orient`]), so it stays on the *user's* anchored
/// edge after the device rotates. Unified anchor-aware model so the status
/// bar, taskbar and IME all rotate coherently (no longer assumes the bars
/// stay at the physical top/bottom):
///
/// - status bar (`Top`)      → the USER's TOP edge, thickness `sb`.
/// - taskbar (`BottomBar`)   → the USER's BOTTOM edge, thickness `tb`.
/// - IME (`Bottom`)          → the USER's BOTTOM edge, thickness = the
///                             keyboard depth, offset `tb` inward so it sits
///                             just above the taskbar.
///
/// The panel buffer is fixed portrait `pw × ph`. Which physical edge is the
/// user's bottom depends on `orient` (device-verified handedness): 0→South,
/// 3→North, 4→West, 7→East; the user's top is the opposite edge. A strip is
/// `th` thick along the edge normal, full-span along the edge, pushed `off`
/// px inward. For 90°/270° the strip is vertical (`w = th, h = ph`); after
/// the host resizes the GL buffer to that, the renderer's dihedral transform
/// swaps logical dims so the guest re-lays-out landscape-wide.
///
/// `None` (fullscreen) never rotates its rect (gated by `is_overlay`).
///
/// Handedness: if a strip lands on the wrong physical side in landscape on a
/// given panel, swap the `4`/`7` arms in `user_bottom_edge` (mirrors the
/// caveat on [`device_rotation_to_orient`]). Host-side only — no shim rebuild.
fn overlay_rect(mode: OverlayMode, orient: u32, pw: i32, ph: i32, t: i32, sb: i32, tb: i32) -> (i32, i32, i32, i32) {
    // Landscape keyboard depth: scale `t` (sized as a fraction of the long
    // edge, ~42%) by pw/ph so it stays the same fraction of the user's
    // screen height (~600 px) rather than an 83%-of-screen slab. Bars keep
    // their fixed thickness (sb/tb) in any orientation.
    let landscape = matches!(orient, 4 | 7);
    let ime_depth = if landscape {
        (((t as i64) * (pw as i64)) / (ph as i64).max(1)).max(1) as i32
    } else {
        t
    };
    // (at the user's bottom edge?, thickness, inward offset)
    let (at_bottom, th, off) = match mode {
        OverlayMode::Top       => (false, sb, 0),         // status bar — user top
        OverlayMode::BottomBar => (true,  tb, 0),         // taskbar — user bottom
        OverlayMode::Bottom    => (true,  ime_depth, tb), // IME — above the taskbar
        OverlayMode::None      => return (0, 0, pw, ph),  // fullscreen — no flip
    };
    let user_bottom_edge = match orient {
        0 => Edge::South, // portrait — physical bottom
        3 => Edge::North, // 180°
        4 => Edge::West,  // 90°  — physical left
        7 => Edge::East,  // 270° — physical right
        _ => Edge::South,
    };
    let edge = if at_bottom {
        user_bottom_edge
    } else {
        match user_bottom_edge { // user top = opposite edge
            Edge::South => Edge::North,
            Edge::North => Edge::South,
            Edge::West  => Edge::East,
            Edge::East  => Edge::West,
        }
    };
    // Place a `th`-thick, full-span strip `off` px inward from `edge`.
    match edge {
        Edge::South => (0, ph - off - th, pw, th),
        Edge::North => (0, off, pw, th),
        Edge::West  => (off, 0, th, ph),
        Edge::East  => (pw - off - th, 0, th, ph),
    }
}

/// The real render loop: instantiate the component and drive `render_frame`.
fn run_cwasm_loop(
    engine: &wasmtime::Engine,
    loaded: LoadedApp,
    renderer: crate::canvas_impl::SkiaRenderer,
    sf: crate::sf_surface::SfSurface,
    // Task 62 — the surface's anchor (fullscreen vs which overlay edge).
    // Drives the rotated-rect geometry flip for overlays.
    mode: OverlayMode,
    // Task 43/62 — auto-follow device screen rotation. True for the
    // fullscreen app (unless locked) and for overlays whose manifest opts
    // in (`orientation = "auto"`); false otherwise.
    enable_rotation: bool,
    // Task 63 — this app explicitly declared `orientation = "locked"`. When
    // it's a foreground fullscreen app it publishes the system orientation
    // lock so the chrome (overlays) stays portrait too.
    orientation_locked: bool,
    // Task 55 — SurfaceFlinger layer for the Foreground role. System
    // chrome (status bar / IME overlays) uses i32::MAX; fullscreen apps
    // use a lower band so chrome always composites above them (otherwise
    // a newly-launched app, created after the status bar, wins the
    // equal-layer tie-break and covers it).
    fg_layer: i32,
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

    let inst = loaded.instantiate(&mut store)?;
    let skiko = inst.skiko;
    let ime_events = inst.ime_events;
    // Task 64 — Some(...) only if the guest exports my:skiko-gfx/frame-pacing.
    let frame_pacing = inst.frame_pacing;
    // Task 64 follow-up — per-app render-rate cap. Resolution order:
    // WART_MAX_FPS env (global, for testing) > package.toml `max_fps` > 60.
    // Enforced host-side as a floor on the render interval, so it caps every
    // app (Compose / dioxus / canvas) with no guest code. Input is still
    // polled at POLL_MS regardless, so touch latency is unchanged.
    let target_fps: u32 = std::env::var("WART_MAX_FPS")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|n| *n >= 1 && *n <= 240)
        .unwrap_or_else(|| loaded.max_fps());
    let frame_interval = std::time::Duration::from_millis((1000 / target_fps.max(1)) as u64);
    log::info!(
        "standalone: render cap {target_fps} fps (min interval {} ms)",
        frame_interval.as_millis()
    );
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
    // Hold the bound socket path so the graceful-shutdown break below
    // can unlink it (task 54 part B). SIGKILL (the LMK case) skips this
    // — covered instead by the arbiter's death-driven unlink + the
    // zygote's startup sweep.
    let ime_inbound_sock: Option<String> = match crate::ime_inbound::spawn_listener() {
        Ok(path) => {
            log::info!("standalone: ime-inbound listening on {path}");
            Some(path)
        }
        Err(e) => {
            log::warn!("standalone: ime-inbound spawn failed: {e:#}");
            None
        }
    };

    // Task 43 — runtime screen orientation. Read the native Device
    // Orientation HAL sensor (android.sensor.device_orientation, type 27,
    // on-change) — the SAME source WMS's WindowOrientationListener uses,
    // but consumed ART-free via the rsbinder sensorservice path. On a
    // rotation change we recompute the renderer's content-pre-rotation
    // matrix + swapped logical dims and re-issue on_resize. For the
    // fullscreen app the physical SurfaceFlinger buffer is never touched
    // (no shim, no EGL resize); for an overlay (task 62) we additionally
    // flip the overlay's anchored rect via the shim so a bottom strip
    // becomes a side strip in landscape (see the rotation block below).
    //
    // A manual `WART_ORIENT` override (read once in the renderer ctor)
    // disables auto-follow — useful for forcing a fixed orientation in a
    // stationary test. Overlays rotate only when their manifest declares
    // `orientation = "auto"` (task 62); otherwise they stay portrait.
    let orient_sensor: Option<u32> =
        if enable_rotation && std::env::var("WART_ORIENT").is_err() {
            match crate::sensors_impl::device_orientation_handle() {
                Some(h) => {
                    let ok = crate::sensors_impl::enable_sensor(h, 5);
                    log::info!("standalone: Device Orientation sensor handle={h} enabled={ok} — auto-rotate on");
                    if ok { Some(h) } else { None }
                }
                None => {
                    log::info!("standalone: no Device Orientation sensor — auto-rotate off");
                    None
                }
            }
        } else {
            log::info!("standalone: auto-rotate disabled (overlay or WART_ORIENT set)");
            None
        };

    // ── Render loop — mirrors WindowEvent::RedrawRequested, no winit ─────
    // (Frame pacing is task 64's `next_render_at` gate, set up below.)
    let mut frame: u64 = 0;
    // Task 62 — the overlay's strip thickness (its portrait height). Seeded
    // from the created surface; updated whenever the guest requests a new
    // overlay height. Feeds `overlay_rect` so a rotation re-derives the
    // side-strip geometry at the current thickness. Unused for fullscreen.
    let mut strip_t: i32 = sf.height;
    // Task 63 — the device's latest orientation from the on-change sensor.
    // Tracked so the target orient can be re-derived when the system lock
    // toggles (overlays), not only when the device physically turns.
    let mut last_dev_orient: u32 = 0;
    // Task 64 — on-demand rendering. The cheap input/IME/scheduler poll
    // still runs every iteration (≤POLL_MS latency), but the expensive
    // render-frame + buffer swap is gated: it fires only when something
    // changed this iteration (`dirty`) or the guest's requested deadline
    // (`next_render_at`) has arrived. A guest that exports `frame-pacing`
    // drives `next_render_at`; one that doesn't falls back to POLL_MS (the
    // legacy unconditional 60 fps). IDLE_CAP bounds how long a fully static
    // guest can sleep, so e.g. the status-bar clock still ticks ~1/sec.
    const IDLE_CAP_MS: u64 = 1000;
    const POLL_MS: u64 = 16;
    let mut next_render_at = std::time::Instant::now();
    loop {
        // Task 64 — set true by any event source below that did real work
        // this iteration; forces a render regardless of the pacing deadline.
        let mut dirty = false;
        // Step 5 — SIGTERM / SIGINT / SIGHUP from launcher trap or operator.
        if crate::lifecycle_standalone::should_shutdown() {
            log::info!("standalone: shutdown signal — exiting render loop");
            // Task 54 part B — graceful-path unlink of our control
            // socket so it doesn't linger after a clean exit.
            if let Some(ref p) = ime_inbound_sock {
                let _ = std::fs::remove_file(p);
                log::info!("standalone: removed ime-inbound socket {p}");
            }
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
            dirty = true; // task 64 — z-order/lifecycle change needs a frame
            match cur_role {
                AppRole::Foreground => {
                    sf.set_layer(fg_layer);
                    sf.set_visible(true);
                    sf.request_focus();
                    // Task 63 — the foreground fullscreen app owns the system
                    // orientation lock: publish ours so the chrome overlays
                    // honor it. (Overlays never write it — they only read.)
                    if mode == OverlayMode::None {
                        publish_orientation_lock(orientation_locked);
                    }
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
                AppRole::OverlayBehind => {
                    // Task 47 step 3c. Stays visible (so the cursor in
                    // the focused editor keeps blinking), demoted in z
                    // (so the IME overlay panel composites on top), and
                    // lifecycle stays Resumed (no Paused fire — the
                    // editor needs to keep rendering text mutations
                    // from the IME). Layer 0 is the same z as the
                    // background pool; IME at i32::MAX or MAX-1 wins.
                    sf.set_layer(0);
                    sf.set_visible(true);
                    let target = bindings::my::skiko_gfx::lifecycle::State::Resumed;
                    if store.data().lifecycle.current != target {
                        store.data_mut().lifecycle.current = target;
                        if let Err(e) = skiko
                            .my_skiko_gfx_renderer()
                            .call_on_lifecycle_changed(&mut store, target as u32)
                        {
                            log::warn!("standalone: on_lifecycle_changed(overlay-behind→Resumed) failed: {e:#}");
                        }
                    }
                }
            }
            last_role = Some(cur_role);
        }

        // Task 47 step 3c — drain any pending overlay-height request from
        // the `my:skiko-gfx/keyboard.request-overlay-height` Host impl.
        // No-op on fullscreen surfaces (the SfSurface gate inside
        // `resize_overlay` warns); on overlay surfaces, this re-issues
        // setSize/setPosition + ANativeWindow_setBuffersGeometry so the
        // next frame draws at the new dimensions. EGL/Skia will pick up
        // the new size via the producer-side geometry update.
        if let Some(new_h) = crate::sf_surface::take_pending_overlay_resize() {
            // Task 62 — the guest's requested thickness. Route it through
            // `overlay_rect` (not the raw bottom-anchored `resize_overlay`)
            // so the keyboard lands in the SAFE AREA above the taskbar /
            // below the status bar at the CURRENT orientation — both in
            // portrait (re-anchor above the taskbar) and while rotated
            // (re-derive the side strip at the new depth).
            strip_t = new_h;
            if sf.is_overlay() {
                let orient = store.data().renderer.current_orient;
                let sb = crate::status_impl::status_bar_height_px() as i32;
                let tb = taskbar_height_px() as i32;
                let (rx, ry, rw, rh) =
                    overlay_rect(mode, orient, sf.panel_w, sf.panel_h, new_h, sb, tb);
                if sf.set_overlay_geometry(rx, ry, rw, rh, rw, rh) {
                    dirty = true; // task 64 — geometry change needs a frame
                    store.data_mut().renderer.resize(rw as u32, rh as u32);
                    let (lw, lh) = {
                        let r = &store.data().renderer;
                        (r.logical_width, r.logical_height)
                    };
                    if let Err(e) = skiko
                        .my_skiko_gfx_renderer()
                        .call_on_resize(&mut store, lw, lh)
                    {
                        log::warn!("standalone: overlay-resize on_resize({lw}x{lh}) failed: {e:#}");
                    }
                    log::info!(
                        "standalone: overlay resize → rect ({rx},{ry},{rw},{rh}) logical {lw}x{lh}"
                    );
                }
            }
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
                    dirty = true; // task 64 — screen on/off lifecycle change
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

        // Task 43/62/63 — apply any screen-rotation change. The HAL sensor
        // is on-change, so `poll_device_rotation` returns Some only right
        // after the device physically rotates; we cache it in
        // `last_dev_orient`. The TARGET orient is the device orient, except
        // an overlay honors the system orientation lock (task 63) and stays
        // portrait while a locked fullscreen app is foreground. Driven by
        // target-vs-current so it also re-applies when the lock toggles
        // without the device moving. Fullscreen apps don't read the lock —
        // they ARE the locker (a locked one has no sensor, stays portrait).
        if let Some(h) = orient_sensor {
            if let Some(rot) = crate::sensors_impl::poll_device_rotation(h) {
                last_dev_orient = device_rotation_to_orient(rot);
            }
            let locked = sf.is_overlay() && orientation_lock_active();
            let orient = if locked { 0 } else { last_dev_orient };
            if orient != store.data().renderer.current_orient {
                dirty = true; // task 64 — rotation needs a frame
                // Task 62 — for an overlay, the anchored rect itself must
                // flip (bottom strip ⇄ side strip). Do the SF move/resize +
                // GL-buffer resize BEFORE set_orientation, which reads the
                // buffer dims. Fullscreen leaves the buffer alone (task 43).
                if sf.is_overlay() {
                    let sb = crate::status_impl::status_bar_height_px() as i32;
                    let tb = taskbar_height_px() as i32;
                    let (rx, ry, rw, rh) =
                        overlay_rect(mode, orient, sf.panel_w, sf.panel_h, strip_t, sb, tb);
                    if sf.set_overlay_geometry(rx, ry, rw, rh, rw, rh) {
                        store.data_mut().renderer.resize(rw as u32, rh as u32);
                        log::info!(
                            "standalone: overlay rect flip → orient {orient}{} rect ({rx},{ry},{rw},{rh})",
                            if locked { " (locked)" } else { "" }
                        );
                    }
                }
                if store.data_mut().renderer.set_orientation(orient) {
                    let (lw, lh) = {
                        let r = &store.data().renderer;
                        (r.logical_width, r.logical_height)
                    };
                    log::info!(
                        "standalone: orient change → {orient}{} logical {lw}x{lh}",
                        if locked { " (system lock)" } else { "" }
                    );
                    if let Err(e) = skiko
                        .my_skiko_gfx_renderer()
                        .call_on_resize(&mut store, lw, lh)
                    {
                        log::warn!("standalone: rotation on_resize({lw}x{lh}) failed: {e:#}");
                    }
                }
            }
        }

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        // Drain InputFlinger events and dispatch them to the guest. The
        // shim's input channel is non-blocking; this is the standalone
        // equivalent of winit's touch/key events (task 33 Step 3).
        let mut input_buf = [crate::sf_surface::SfInputEvent::default(); 32];
        for ev in sf.poll_input(&mut input_buf) {
            dirty = true; // task 64 — any input (touch/key) wants a frame
            if ev.kind <= 3 {
                // Task 43 — touch coords arrive in physical-buffer space.
                // When the content is rotated, map them back into logical
                // space via the inverse of the renderer's base_matrix so
                // taps land where the (rotated) UI actually drew. Identity
                // matrix (orient 0) ⇒ inverse is identity ⇒ no-op, so the
                // common unrotated path is unchanged.
                let (lx, ly) = {
                    let base = store.data().renderer.base_matrix;
                    match base.invert() {
                        Some(inv) => {
                            let p = inv.map_point((ev.x, ev.y));
                            (p.x, p.y)
                        }
                        None => (ev.x, ev.y),
                    }
                };
                if let Err(e) = crate::input::dispatch_pointer_v2(
                    &skiko, &mut store, ev.kind as u8,
                    ev.pointer_id as u32, lx, ly, ev.pressure,
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
            dirty = true; // task 64 — IME key/editor event wants a frame
            match ev {
                crate::ime_inbound::InboundEvent::KeyEvent { code_point, key_id, action } => {
                    if let Err(e) = crate::input::dispatch_key_v2(
                        &skiko, &mut store, action, code_point, key_id,
                    ) {
                        log::warn!("standalone: dispatch_key_v2 (ime-inbound) failed: {e:#}");
                    }
                }
                // Task 49 step 1a — IME-bound events. Only meaningful
                // for hosts running in IME role (--standalone-overlay).
                // Step 1a logs; step 1b adds the call into the guest's
                // exported war:ime/ime.on-editor-attached(info).
                crate::ime_inbound::InboundEvent::EditorAttached { info } => {
                    let Some(ie) = ime_events.as_ref() else {
                        log::warn!(
                            "ime-inbound: editor-attached received but host has \
                             no IME bindings (component doesn't export war:ime/ime)"
                        );
                        continue;
                    };
                    // Convert the wire string → typed WIT enum. Unknown
                    // tags fall back to Text — defensive.
                    let wit_input_type = match info.input_type.as_str() {
                        "text"           => crate::ime_bindings::war::ime::types::InputType::Text,
                        "number"         => crate::ime_bindings::war::ime::types::InputType::Number,
                        "phone"          => crate::ime_bindings::war::ime::types::InputType::Phone,
                        "email"          => crate::ime_bindings::war::ime::types::InputType::Email,
                        "url"            => crate::ime_bindings::war::ime::types::InputType::Url,
                        "password"       => crate::ime_bindings::war::ime::types::InputType::Password,
                        "multiline-text" => crate::ime_bindings::war::ime::types::InputType::MultilineText,
                        other => {
                            log::warn!(
                                "ime-inbound: unknown input-type {other:?} — defaulting to Text"
                            );
                            crate::ime_bindings::war::ime::types::InputType::Text
                        }
                    };
                    if let Err(e) = ie.war_ime_ime()
                        .call_on_editor_attached(&mut store, wit_input_type)
                    {
                        log::warn!(
                            "ime-inbound: on-editor-attached failed: {e:#}"
                        );
                        if e.downcast_ref::<wasmtime::ThrownException>().is_some() {
                            if let Some(exn_ref) = store.take_pending_exception() {
                                let _ = log_kotlin_exception_msg(&mut store, &exn_ref);
                            }
                        }
                    } else {
                        log::info!(
                            "ime-inbound: dispatched on-editor-attached input-type={:?} \
                             (hint/text dropped on wire — see ime.wit)",
                            info.input_type,
                        );
                    }
                }
                crate::ime_inbound::InboundEvent::EditorDetached => {
                    let Some(ie) = ime_events.as_ref() else {
                        log::warn!(
                            "ime-inbound: editor-detached received but host has \
                             no IME bindings"
                        );
                        continue;
                    };
                    if let Err(e) = ie.war_ime_ime().call_on_editor_detached(&mut store) {
                        log::warn!("ime-inbound: on-editor-detached failed: {e:#}");
                    } else {
                        log::info!("ime-inbound: dispatched on-editor-detached");
                    }
                }
            }
        }

        // Drain scheduler callbacks whose deadline has passed.
        let due = store.data_mut().scheduler.drain_due(std::time::Instant::now());
        if !due.is_empty() {
            dirty = true; // task 64 — a timer came due (delay()/animation tick)
        }
        for cb in due {
            if let Err(e) = skiko
                .my_skiko_gfx_renderer()
                .call_on_scheduled_callback(&mut store, cb)
            {
                log::warn!("standalone: on_scheduled_callback({cb}) failed: {e:#}");
            }
        }

        // Task 64 — gate the expensive render-frame + buffer swap. Render
        // when something changed this iteration (`dirty`), the guest's pacing
        // deadline arrived, or during the first few warm-up frames. `dirty`
        // (input / IME / lifecycle) always renders promptly — the fps cap must
        // NOT add latency to a discrete tap (a down-then-up within one frame
        // interval would otherwise defer the click to the idle deadline). The
        // cap instead floors the TIMED/animation cadence via `next_render_at`
        // below, so unconditional/animating guests stay at `target_fps`.
        if frame < 3 || std::time::Instant::now() >= next_render_at || dirty {
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

            // Ask the guest how long it may sleep before the next frame.
            // No frame-pacing export ⇒ 0 (render every interval — the legacy
            // unconditional path, now rate-limited by the fps cap below).
            let guest_delay = match frame_pacing.as_ref() {
                Some(fp) => fp
                    .my_skiko_gfx_frame_pacing()
                    .call_next_frame_delay(&mut store)
                    .unwrap_or(0)
                    .min(IDLE_CAP_MS as u32) as u64,
                None => 0,
            };
            // Floor the next-frame delay by the fps cap so an animating /
            // unconditional guest never renders faster than `target_fps`.
            let delay_ms = guest_delay.max(frame_interval.as_millis() as u64);
            next_render_at =
                std::time::Instant::now() + std::time::Duration::from_millis(delay_ms);

            if frame <= 3 || frame % 600 == 0 {
                log::info!("standalone: rendered frame {frame}");
            }
        }

        frame += 1;
        // Task 46 step 4 — only the foreground app fights for focus.
        // Background apps that keep stealing focus would defeat the
        // arbiter's policy + spam the launcher. Frequency unchanged
        // (~1 s) when foreground.
        if frame % focus_refresh_interval == 0 && crate::app_role::is_foreground() {
            sf.request_focus();
        }

        // Task 64 — cheap poll cadence. Sleep until the next render is due,
        // but never longer than POLL_MS so idle input latency stays ≤16 ms
        // while the expensive render is skipped.
        let nap = next_render_at
            .saturating_duration_since(std::time::Instant::now())
            .min(std::time::Duration::from_millis(POLL_MS));
        if !nap.is_zero() {
            std::thread::sleep(nap);
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

/// Walk a Kotlin Throwable's anyref to extract `.message: String?` and
/// log it. Mirrors the render_frame error path in lib.rs:392-440 —
/// kept here as a private helper so the ime-inbound dispatch can use
/// the same exception-payload format. Returns Ok(()) regardless;
/// failures to walk the struct are logged via log::error.
///
/// Kotlin/Wasm Throwable struct layout (offsets):
///   0=vtable 1=itable 2=rtti 3=_hashCode 4=message 5=cause 6=suppressed
/// Kotlin/Wasm String struct layout:
///   0=vtable 1=itable 2=rtti 3=_hashCode 4=leftIfInSum 5=length 6=_chars
/// _chars is an Array<i16> of UTF-16 code units.
fn log_kotlin_exception_msg(
    store: &mut Store<HostState>,
    exn_ref: &wasmtime::ExnRef,
) -> anyhow::Result<()> {
    use anyhow::anyhow;
    use wasmtime::Val;
    let throwable_val = exn_ref.field(&mut *store, 0)?;
    let throwable_anyref = throwable_val.unwrap_anyref()
        .ok_or_else(|| anyhow!("exn field 0 null/not anyref"))?
        .clone();
    let throwable_struct = throwable_anyref.unwrap_struct(&mut *store)?;
    let msg_val = throwable_struct.field(&mut *store, 4)?;
    let msg_anyref = match msg_val.unwrap_anyref() {
        Some(a) => a.clone(),
        None => {
            log::error!("  exception message: <null>");
            return Ok(());
        }
    };
    let str_struct = msg_anyref.unwrap_struct(&mut *store)?;
    let len_val = str_struct.field(&mut *store, 5)?;
    let length = match len_val {
        Val::I32(i) => i as usize,
        other => return Err(anyhow!("length not i32: {:?}", other)),
    };
    let chars_val = str_struct.field(&mut *store, 6)?;
    let chars_anyref = chars_val.unwrap_anyref()
        .ok_or_else(|| anyhow!("_chars null/not anyref"))?
        .clone();
    let chars_array = chars_anyref.unwrap_array(&mut *store)?;
    let mut out = Vec::<u16>::with_capacity(length);
    for v in chars_array.elems(&mut *store)?.take(length) {
        let c = match v { Val::I32(i) => i as u16, _ => 0 };
        out.push(c);
    }
    log::error!("  exception message: {}", String::from_utf16_lossy(&out));
    Ok(())
}
