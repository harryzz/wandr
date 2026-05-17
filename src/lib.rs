mod egl;
mod canvas_impl;
mod paragraph_impl;
mod window_impl;
mod scheduler_impl;
mod text_segmentation_impl;
mod lifecycle_impl;
mod haptics_impl;
mod lights_impl;
mod power_impl;
mod thermal_impl;
mod sensors_impl;
mod locale_impl;
mod clipboard_impl;
mod pointer_icon_impl;
mod input;
mod binder;
mod binder_aidl;
#[cfg(target_os = "android")]
mod bionic_compat;

mod bindings {
    wasmtime::component::bindgen!({
        path: "../wit/skiko-gfx.wit",
        world: "skiko-ui",
    });
}

use winit::{
    application::ApplicationHandler,
    event::{ElementState, TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey, PhysicalKey},
    window::{Window, WindowId},
};
use std::sync::Arc;
use wasmtime::{Engine, Store};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

pub struct HostState {
    pub renderer:  canvas_impl::SkiaRenderer,
    pub scheduler: scheduler_impl::SchedulerState,
    pub lifecycle: lifecycle_impl::LifecycleState,
    pub clipboard: Option<String>,
    pub wasi:      WasiCtx,
    pub table:     ResourceTable,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

pub struct App {
    window:          Option<Arc<Window>>,
    engine:          Engine,
    component:       Option<Component>,
    store:           Option<Store<HostState>>,
    bindings:        Option<bindings::SkikoUi>,
    // Renderer owned directly when running without a WASM component.
    test_renderer:   Option<canvas_impl::SkiaRenderer>,
    last_cursor:     (f32, f32),
}

impl App {
    fn make_engine() -> Engine {
        let mut config = wasmtime::Config::new();
        config.wasm_component_model(true);
        config.wasm_gc(true);
        config.wasm_function_references(true);
        config.wasm_exceptions(true);
        // Compose's composition + layout passes on a real Material3 tree get
        // deeply recursive (Composer/snapshot diffing + LayoutNode placement).
        // Default 1 MiB wasm stack overflows; bump to 4 MiB. async_stack_size
        // defaults to 2 MiB though, and wasmtime requires async_stack > max_wasm_stack;
        // bump async first.
        config.async_stack_size(8 * 1024 * 1024);
        config.max_wasm_stack(4 * 1024 * 1024);
        Engine::new(&config).expect("wasmtime engine init")
    }

    pub fn new() -> Self {
        let engine = Self::make_engine();

        let wasm_path = std::env::args().nth(1)
            .unwrap_or_else(|| "skiko-component.cwasm".to_string());
        log::info!("loading WASM from path: {wasm_path}");

        let component = if wasm_path.ends_with(".cwasm") {
            match unsafe { Component::deserialize_file(&engine, &wasm_path) } {
                Ok(c) => { log::info!("loaded cwasm: {wasm_path}"); Some(c) }
                Err(e) => { log::warn!("no cwasm at {wasm_path}: {e}"); None }
            }
        } else {
            match Component::from_file(&engine, &wasm_path) {
                Ok(c) => { log::info!("loaded wasm: {wasm_path}"); Some(c) }
                Err(e) => { log::warn!("no wasm at {wasm_path}: {e}"); None }
            }
        };

        Self::from_parts(engine, component)
    }

    #[cfg(target_os = "android")]
    pub fn new_from_asset_bytes(bytes: Vec<u8>) -> Self {
        let engine = Self::make_engine();
        let component = match unsafe { Component::deserialize(&engine, &bytes) } {
            Ok(c) => { log::info!("loaded cwasm from asset ({} bytes)", bytes.len()); Some(c) }
            Err(e) => { log::warn!("failed to deserialize asset cwasm: {e}"); None }
        };
        Self::from_parts(engine, component)
    }

    fn from_parts(engine: Engine, component: Option<Component>) -> Self {
        Self {
            window: None,
            engine,
            component,
            store: None,
            bindings: None,
            test_renderer: None,
            last_cursor: (0.0, 0.0),
        }
    }

    fn renderer_mut(&mut self) -> Option<&mut canvas_impl::SkiaRenderer> {
        if let Some(store) = &mut self.store {
            Some(&mut store.data_mut().renderer)
        } else {
            self.test_renderer.as_mut()
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(reason) = binder::init() {
            log::warn!("binder init: {reason} — HAL calls will fall back to sysfs");
        }
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes()
                    .with_title("WASM Android Runtime"))
                .expect("window creation failed"),
        );

        let renderer = canvas_impl::SkiaRenderer::new(window.clone())
            .expect("renderer init failed");

        // Warm resume: a store + bindings are already alive from a previous
        // cold start, but our renderer's EGL surface points at a NativeWindow
        // that Android destroyed when the activity was backgrounded. Swap in
        // a fresh renderer bound to the new NativeWindow, inheriting the
        // old renderer's CPU-side caches (pictures, recorders, text blobs,
        // typefaces, shaders, paragraphs, paragraph-builders + their id
        // counters) so wasm-side handles that Compose still holds remain
        // valid. GPU-resident caches (text image cache, images) are NOT
        // inherited — their textures lived in the dying gr_context.
        // Composition, scheduler state, and lifecycle observers persist
        // because the wasmtime Store is preserved.
        if self.store.is_some() && self.bindings.is_some() {
            log::info!("resumed (warm) — swapping renderer in existing store, inheriting CPU caches");
            let store = self.store.as_mut().unwrap();
            let mut new_renderer = renderer;
            let old_renderer = &mut store.data_mut().renderer;
            new_renderer.inherit_caches_from(old_renderer);
            let _stale = std::mem::replace(old_renderer, new_renderer);
            drop(_stale);
            self.window = Some(window);
            self.set_lifecycle(bindings::my::skiko_gfx::lifecycle::State::Resumed);
            if let Some(w) = &self.window { w.request_redraw(); }
            return;
        }

        log::info!("resumed (cold) — creating store and instantiating component");
        if let Some(component) = &self.component {
            let wasi = WasiCtxBuilder::new().inherit_stdio().build();
            let host = HostState {
                renderer,
                scheduler: scheduler_impl::SchedulerState::default(),
                lifecycle: lifecycle_impl::LifecycleState {
                    current: bindings::my::skiko_gfx::lifecycle::State::Resumed,
                    pending: Some(bindings::my::skiko_gfx::lifecycle::State::Resumed),
                },
                clipboard: None,
                wasi,
                table: ResourceTable::new(),
            };
            let mut store = Store::new(&self.engine, host);

            let mut linker: Linker<HostState> = Linker::new(&self.engine);
            wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
                .expect("add WASI to linker");
            bindings::SkikoUi::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |s| s)
                .expect("add SkikoUi canvas Host to linker");

            let bindings = bindings::SkikoUi::instantiate(
                &mut store, component, &linker,
            ).expect("instantiate component");
            log::info!("WASM component instantiated");

            self.store   = Some(store);
            self.bindings = Some(bindings);
        } else {
            log::info!("no WASM component — running in renderer-test mode");
            self.test_renderer = Some(renderer);
        }
        self.window = Some(window);

        if let Some(w) = &self.window { w.request_redraw(); }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("suspended — dispatching Stopped, dropping window (store kept alive)");
        // Dispatch Stopped through the guest BEFORE releasing the window —
        // wasm-side observers can react while the renderer is still valid.
        self.set_lifecycle(bindings::my::skiko_gfx::lifecycle::State::Stopped);
        // Drop the Window reference so winit / android-activity can release
        // the NativeWindow cleanly. The renderer's EGL surface inside the
        // wasmtime Store will become invalid as a side effect; nothing
        // touches it until resumed() swaps in a fresh one.
        self.window         = None;
        self.test_renderer  = None;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::RedrawRequested => {
                if let (Some(b), Some(s)) = (&self.bindings, self.store.as_mut()) {
                    // No init() call needed — appMain() runs on the first render-frame.
                    let nanos = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos() as u64;

                    // Drain any scheduler callbacks whose deadline has passed.
                    let due = s.data_mut().scheduler.drain_due(std::time::Instant::now());
                    for callback_id in due {
                        if let Err(e) = b.my_skiko_gfx_renderer()
                            .call_on_scheduled_callback(&mut *s, callback_id)
                        {
                            log::warn!("on_scheduled_callback({callback_id}) failed: {e:#}");
                        }
                    }

                    let t0 = std::time::Instant::now();
                    let result = b.my_skiko_gfx_renderer().call_render_frame(&mut *s, nanos);

                    // Fire any pending lifecycle transition AFTER the first
                    // render_frame succeeds (gives appMain a chance to register
                    // its observer before the event arrives).
                    if result.is_ok() {
                        let pending = s.data_mut().lifecycle.pending.take();
                        if let Some(state) = pending {
                            if let Err(e) = b.my_skiko_gfx_renderer()
                                .call_on_lifecycle_changed(&mut *s, state as u32)
                            {
                                log::warn!("on_lifecycle_changed failed: {e:#}");
                            }
                        }
                    }
                    let elapsed = t0.elapsed();
                    {
                        static FRAME_COUNT: std::sync::atomic::AtomicU32 =
                            std::sync::atomic::AtomicU32::new(0);
                        let n = FRAME_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        // Note on continuous-animation leak (~0.4 MB/s in wasm
                        // linear memory under indeterminate ProgressIndicator +
                        // LaunchedEffect+withFrameNanos workloads): wasmtime's
                        // automatic GC heuristic isn't aggressive enough.
                        // Manual `s.gc(None)` every 60-300 frames reduces the
                        // leak ~5x but the per-call cost grows monotonically
                        // (DRC scans retained refs in Kotlin/Wasm-generated
                        // continuations), pushing CPU from ~17% to ~75% over
                        // a minute. Not enabled by default — prefer static
                        // widgets over continuous animations as the practical
                        // mitigation. See feedback_indeterminate_progress_leak.md.
                        if n < 5 {
                            log::info!("render_frame #{n}: {:?} ok={}", elapsed, result.is_ok());
                            if let Err(ref e) = result {
                                log::error!("render_frame #{n} error: {e:#}");
                                if e.downcast_ref::<wasmtime::ThrownException>().is_some() {
                                    if let Some(exn_ref) = s.take_pending_exception() {
                                        // Walk Throwable struct -> message: String?
                                        // Throwable: 0=vtable 1=itable 2=rtti 3=_hashCode 4=message 5=cause 6=suppressed
                                        // String:    0=vtable 1=itable 2=rtti 3=_hashCode 4=leftIfInSum 5=length 6=_chars
                                        // _chars: array of i16 (UTF-16)
                                        let msg = (|| -> anyhow::Result<String> {
                                            use anyhow::anyhow;
                                            use wasmtime::Val;
                                            let throwable_val = exn_ref.field(&mut *s, 0)?;
                                            let throwable_anyref = throwable_val.unwrap_anyref()
                                                .ok_or_else(|| anyhow!("exn field 0 null/not anyref"))?
                                                .clone();
                                            let throwable_struct = throwable_anyref.unwrap_struct(&mut *s)?;
                                            let msg_val = throwable_struct.field(&mut *s, 4)?;
                                            let msg_anyref = match msg_val.unwrap_anyref() {
                                                Some(a) => a.clone(),
                                                None => return Ok("<null message>".into()),
                                            };
                                            let str_struct = msg_anyref.unwrap_struct(&mut *s)?;
                                            let len_val = str_struct.field(&mut *s, 5)?;
                                            let length = match len_val {
                                                Val::I32(i) => i as usize,
                                                other => return Err(anyhow!("length not i32: {:?}", other)),
                                            };
                                            let chars_val = str_struct.field(&mut *s, 6)?;
                                            let chars_anyref = chars_val.unwrap_anyref()
                                                .ok_or_else(|| anyhow!("_chars null/not anyref"))?
                                                .clone();
                                            let chars_array = chars_anyref.unwrap_array(&mut *s)?;
                                            let mut out = Vec::<u16>::with_capacity(length);
                                            for v in chars_array.elems(&mut *s)?.take(length) {
                                                let c = match v { Val::I32(i) => i as u16, _ => 0 };
                                                out.push(c);
                                            }
                                            Ok(String::from_utf16_lossy(&out))
                                        })();
                                        match msg {
                                            Ok(text) => log::error!("  exception message: {text}"),
                                            Err(why) => log::error!("  exception message read failed: {why:#}"),
                                        }
                                    } else {
                                        log::error!("  no pending exception object on store");
                                    }
                                }
                            }
                        }
                    }
                    if let Err(e) = result {
                        let msg = format!("{e:?}");
                        if msg.contains("cannot enter component instance") {
                            // skip — store is poisoned, keep rendering test frame
                        } else {
                            log::error!("render_frame fatal: {msg}");
                            event_loop.exit();
                            return;
                        }
                    }
                } else if let Some(r) = self.renderer_mut() {
                    r.draw_test_frame();
                }
                if let Some(w) = &self.window { w.request_redraw(); }
            }

            WindowEvent::Resized(size) => {
                if let Some(r) = self.renderer_mut() {
                    r.resize(size.width, size.height);
                }
                if let Some(w) = &self.window { w.request_redraw(); }
            }

            WindowEvent::Touch(t) => {
                let kind: u8 = match t.phase {
                    TouchPhase::Started               => 0,
                    TouchPhase::Ended | TouchPhase::Cancelled => 1,
                    TouchPhase::Moved                 => 2,
                };
                // Normalize pressure to 0.0..1.0. winit's Force is either
                // Normalized(f64) already in [0,1], or Calibrated { force,
                // max_possible } where the ratio gives us [0,1].
                let pressure: f32 = match t.force {
                    Some(winit::event::Force::Normalized(p)) => p as f32,
                    Some(winit::event::Force::Calibrated { force, max_possible_force, .. }) => {
                        if max_possible_force > 0.0 {
                            (force / max_possible_force) as f32
                        } else { 0.0 }
                    }
                    None => 0.0,
                };
                let pointer_id: u32 = (t.id & 0xFFFF_FFFF) as u32;
                if let (Some(b), Some(s)) = (&self.bindings, self.store.as_mut()) {
                    let _ = input::dispatch_pointer_v2(
                        b, s, kind, pointer_id,
                        t.location.x as f32, t.location.y as f32,
                        pressure,
                    );
                }
                if let Some(w) = &self.window { w.request_redraw(); }
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.last_cursor = (position.x as f32, position.y as f32);
                if let (Some(b), Some(s)) = (&self.bindings, self.store.as_mut()) {
                    let _ = input::dispatch_pointer_v2(
                        b, s, 2, 0,
                        self.last_cursor.0, self.last_cursor.1,
                        0.0,
                    );
                }
            }

            WindowEvent::MouseInput { state, .. } => {
                let kind: u8 = if state == ElementState::Pressed { 0 } else { 1 };
                let (cx, cy) = self.last_cursor;
                if let (Some(b), Some(s)) = (&self.bindings, self.store.as_mut()) {
                    let _ = input::dispatch_pointer_v2(b, s, kind, 0, cx, cy, 1.0);
                }
                if let Some(w) = &self.window { w.request_redraw(); }
            }

            WindowEvent::KeyboardInput { event, .. } => {
                let kind: u8 = if event.state == ElementState::Pressed { 0 } else { 1 };
                let code: u32 = match event.physical_key {
                    PhysicalKey::Code(c) => c as u32,
                    _ => 0,
                };
                // v2: resolved UTF-32 codepoint + Compose-compatible key-id.
                // First char of `event.text` is the codepoint of the typed
                // character (handles Shift, AltGr, etc.); 0 if no text
                // (modifiers, named keys without a char).
                let code_point: u32 = event
                    .text
                    .as_ref()
                    .and_then(|s| s.chars().next())
                    .map(|c| c as u32)
                    .unwrap_or(0);
                // Numeric IDs match the values upstream Compose's webMain
                // hard-codes for `Key.Backspace`, `Key.Enter`, etc., so the
                // guest can pass `key-id` straight into `Key(keyCode.toLong())`
                // without a translation table.
                let key_id: u32 = match &event.logical_key {
                    Key::Named(NamedKey::Backspace)  => 8,
                    Key::Named(NamedKey::Tab)        => 9,
                    Key::Named(NamedKey::Enter)      => 13,
                    Key::Named(NamedKey::Escape)     => 27,
                    Key::Named(NamedKey::Space)      => 32,
                    Key::Named(NamedKey::PageUp)     => 33,
                    Key::Named(NamedKey::PageDown)   => 34,
                    Key::Named(NamedKey::End)        => 35,
                    Key::Named(NamedKey::Home)       => 36,
                    Key::Named(NamedKey::ArrowLeft)  => 37,
                    Key::Named(NamedKey::ArrowUp)    => 38,
                    Key::Named(NamedKey::ArrowRight) => 39,
                    Key::Named(NamedKey::ArrowDown)  => 40,
                    Key::Named(NamedKey::Insert)     => 45,
                    Key::Named(NamedKey::Delete)     => 46,
                    _ => 0,  // Unknown — guest falls back to code-point if non-zero
                };
                if let (Some(b), Some(s)) = (&self.bindings, self.store.as_mut()) {
                    let _ = input::dispatch_key(b, s, kind, code);
                    let _ = input::dispatch_key_v2(b, s, kind, code_point, key_id);
                }
            }

            // winit collapses Activity.onResume/onPause/onStart/onStop down to
            // its own resumed/suspended (which actually track NativeWindow
            // create/terminate, not the Activity state). But Android dispatches
            // a Focus change adjacent to onPause/onResume — LostFocus fires
            // immediately before onPause, GainedFocus immediately after
            // onResume. Use this signal to emit Paused / Resumed transitions
            // between the cold-start Resumed (host::resumed) and the eventual
            // Stopped (host::suspended). The bridge in test-app advances the
            // LifecycleRegistry through CREATED → STARTED for free when state
            // increases, so the guest sees ON_PAUSE / ON_RESUME events in the
            // right order.
            WindowEvent::Focused(focused) => {
                self.set_lifecycle(if focused {
                    bindings::my::skiko_gfx::lifecycle::State::Resumed
                } else {
                    bindings::my::skiko_gfx::lifecycle::State::Paused
                });
            }

            _ => {}
        }
    }
}

impl App {
    /// Dispatch a host-driven lifecycle transition into the guest. No-op if
    /// the new state matches what the guest last saw (lifecycle events are
    /// edge-triggered, not level-triggered). Avoids spamming
    /// on_lifecycle_changed when winit raises the same window-focus state
    /// multiple times.
    #[cfg(target_os = "android")]
    fn set_lifecycle(&mut self, state: bindings::my::skiko_gfx::lifecycle::State) {
        if let (Some(b), Some(s)) = (&self.bindings, self.store.as_mut()) {
            if s.data().lifecycle.current == state {
                return;
            }
            s.data_mut().lifecycle.current = state;
            if let Err(e) = b.my_skiko_gfx_renderer()
                .call_on_lifecycle_changed(&mut *s, state as u32)
            {
                log::warn!("on_lifecycle_changed({state:?}) failed: {e:#}");
            }
        }
    }

    #[cfg(not(target_os = "android"))]
    fn set_lifecycle(&mut self, _state: bindings::my::skiko_gfx::lifecycle::State) {}
}

#[cfg(target_os = "android")]
fn load_asset_bytes(app: &winit::platform::android::activity::AndroidApp, name: &str) -> Option<Vec<u8>> {
    use std::ffi::CString;
    use std::io::Read;
    let mgr = app.asset_manager();
    let cname = CString::new(name).ok()?;
    let mut asset = mgr.open(&cname)?;
    let len = asset.length();
    let mut buf = Vec::with_capacity(len);
    asset.read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Search the filesystem for a .cwasm file.
/// Priority: public Downloads (drop file via MTP/file-manager) →
///           app-owned external dir (adb push, no permission needed) →
///           APK asset fallback.
///
/// Deploy via MTP or adb:
///   adb push skiko-component.cwasm /sdcard/Download/skiko-component.cwasm
#[cfg(target_os = "android")]
fn find_cwasm_on_filesystem(app: &winit::platform::android::activity::AndroidApp) -> Option<Vec<u8>> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();

    // 1. Public Downloads — accessible via MTP or any file manager, no special
    //    permissions needed on API 29+ (scoped storage still allows reads from
    //    Download/ when the app holds READ_EXTERNAL_STORAGE or targets API 33+).
    for path in &[
        "/sdcard/Download/skiko-component.cwasm",
        "/sdcard/Downloads/skiko-component.cwasm",
        "/storage/emulated/0/Download/skiko-component.cwasm",
    ] {
        candidates.push(std::path::PathBuf::from(path));
    }

    // 2. App-owned external files dir — never needs a permission grant.
    //    adb push skiko-component.cwasm /sdcard/Android/data/com.example.wasmruntime/files/
    if let Some(ext) = app.external_data_path() {
        candidates.push(ext.join("skiko-component.cwasm"));
    }

    for path in &candidates {
        match std::fs::read(path) {
            Ok(bytes) => {
                log::info!("found cwasm on filesystem: {} ({} bytes)", path.display(), bytes.len());
                return Some(bytes);
            }
            Err(e) => {
                log::debug!("cwasm not at {}: {e}", path.display());
            }
        }
    }
    None
}

#[cfg(target_os = "android")]
#[no_mangle]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
    );
    log::info!("android_main start");

    // Priority: filesystem (hot-reload) → APK assets → file arg
    let mut runner = if let Some(bytes) = find_cwasm_on_filesystem(&app) {
        log::info!("using cwasm from filesystem ({} bytes)", bytes.len());
        App::new_from_asset_bytes(bytes)
    } else if let Some(bytes) = load_asset_bytes(&app, "skiko-component.cwasm") {
        log::info!("loaded skiko-component.cwasm from APK assets ({} bytes)", bytes.len());
        App::new_from_asset_bytes(bytes)
    } else {
        log::warn!("no cwasm found on filesystem or in assets — trying file arg");
        App::new()
    };

    let event_loop = EventLoop::builder()
        .with_android_app(app)
        .build()
        .unwrap();
    event_loop.run_app(&mut runner).unwrap();
}

#[cfg(not(target_os = "android"))]
pub fn run() {
    env_logger::init();
    log::info!("desktop start");
    let event_loop = EventLoop::new().unwrap();
    event_loop.run_app(&mut App::new()).unwrap();
}
