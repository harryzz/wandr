//! `dlopen` wrapper for `libsf_surface.so` — the task-33 libgui surface shim.
//!
//! wart-host's cargo/NDK cross-compile cannot link `libgui` (Android's
//! private platform C++ library). Instead the shim is built in-tree as a
//! soong `cc_library_shared` (see `cpp/sf_surface.{cpp,bp}`) and loaded here
//! at runtime via `dlopen` — its `libgui`/`libui`/… dependencies resolve
//! from `/system/lib64` on the device. See memory
//! `project-boot-model-libgui-build` and `tasks/33-boot-model-bringup.md`.

use anyhow::{ensure, Result};
use std::ffi::{c_void, CString};

/// `ANativeWindow* sf_create_fullscreen_surface(int32_t*, int32_t*, uint32_t*)`.
type CreateFn = unsafe extern "C" fn(*mut i32, *mut i32, *mut u32) -> *mut c_void;
/// `int32_t sf_input_poll(SfInputEvent*, int32_t)`.
type InputPollFn = unsafe extern "C" fn(*mut SfInputEvent, i32) -> i32;
/// `uint32_t sf_query_transform_hint(void)`.
type QueryHintFn = unsafe extern "C" fn() -> u32;
/// `int32_t sf_request_focus(void)`.
type RequestFocusFn = unsafe extern "C" fn() -> i32;
/// `int32_t sf_set_layer(int32_t z)` — task 46 step 4/5.
type SetLayerFn = unsafe extern "C" fn(i32) -> i32;
/// `int32_t sf_set_visible(int32_t visible)` — task 46 step 4/5.
type SetVisibleFn = unsafe extern "C" fn(i32) -> i32;

/// POD input event drained from the shim's InputFlinger channel. Mirrors
/// `struct SfInputEvent` in `cpp/sf_surface.{cpp,h}` — keep all three in sync.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SfInputEvent {
    /// 0=down 1=up 2=move 3=scroll  10=key-down 11=key-up.
    pub kind: i32,
    /// Multi-touch pointer id (0..N); 0 for key events.
    pub pointer_id: i32,
    pub x: f32,
    pub y: f32,
    /// Normalized pressure 0.0..1.0; 0 for key events.
    pub pressure: f32,
    /// `AKEYCODE_*` for key events; 0 otherwise.
    pub key_code: i32,
    /// `AMETA_*` shift/alt/ctrl bitmask for key events; 0 otherwise.
    pub meta_state: i32,
}

/// A fullscreen surface allocated from SurfaceFlinger by `libsf_surface.so`.
/// Keeps the `dlopen` handle for the process lifetime — unloading the shim
/// would invalidate `native_window`.
pub struct SfSurface {
    _handle: *mut c_void,
    /// `ANativeWindow*` — hand straight to `EglContext::new`.
    pub native_window: *mut c_void,
    pub width: i32,
    pub height: i32,
    /// SurfaceFlinger display rotation the shim applied (ui::Rotation 0..3);
    /// the renderer pairs its base transform with this. See task 33.
    pub transform: u32,
    /// `sf_input_poll` — `None` if the shim predates task-33 Step 3.
    input_poll: Option<InputPollFn>,
    /// `sf_query_transform_hint` — `None` if the shim predates the task-33
    /// orientation fix; query it only *after* EGL has connected the producer.
    query_hint: Option<QueryHintFn>,
    /// `sf_request_focus` — `None` if the shim predates standalone key
    /// support; the host calls this periodically to keep wart focused.
    request_focus: Option<RequestFocusFn>,
    /// `sf_set_layer` — `None` if the shim predates task 46. When the
    /// arbiter (step 4) demotes an app to background it pushes z to 0;
    /// promotion to foreground pulls z back to `i32::MAX`. Until the
    /// .so is rebuilt on the AOSP a-03 host, this stays `None` and
    /// callers fall back to "no z-order control" semantics.
    set_layer: Option<SetLayerFn>,
    /// `sf_set_visible` — `None` if the shim predates task 46. Backs
    /// the cheap "hide while background / show on foreground" path
    /// (the layer stays allocated, BBQ keeps the last frame).
    set_visible: Option<SetVisibleFn>,
}

impl SfSurface {
    /// `dlopen` the shim at `so_path` and create a fullscreen surface.
    pub fn create(so_path: &str) -> Result<Self> {
        unsafe {
            let path = CString::new(so_path)?;
            let handle = libc::dlopen(path.as_ptr(), libc::RTLD_NOW);
            ensure!(!handle.is_null(), "dlopen({so_path}) failed");

            let name = CString::new("sf_create_fullscreen_surface").unwrap();
            let sym = libc::dlsym(handle, name.as_ptr());
            ensure!(
                !sym.is_null(),
                "dlsym sf_create_fullscreen_surface failed in {so_path}"
            );
            let create: CreateFn = std::mem::transmute(sym);

            // sf_input_poll is optional — a shim without it just yields no input.
            let poll_name = CString::new("sf_input_poll").unwrap();
            let poll_sym = libc::dlsym(handle, poll_name.as_ptr());
            let input_poll: Option<InputPollFn> = if poll_sym.is_null() {
                None
            } else {
                Some(std::mem::transmute(poll_sym))
            };

            // sf_query_transform_hint is optional too — an older shim leaves
            // the renderer to fall back on its dims-swapped heuristic.
            let hint_name = CString::new("sf_query_transform_hint").unwrap();
            let hint_sym = libc::dlsym(handle, hint_name.as_ptr());
            let query_hint: Option<QueryHintFn> = if hint_sym.is_null() {
                None
            } else {
                Some(std::mem::transmute(hint_sym))
            };

            let focus_name = CString::new("sf_request_focus").unwrap();
            let focus_sym = libc::dlsym(handle, focus_name.as_ptr());
            let request_focus: Option<RequestFocusFn> = if focus_sym.is_null() {
                None
            } else {
                Some(std::mem::transmute(focus_sym))
            };

            // Task 46 step 4/5 — z-order + visibility toggles. Optional
            // (older shim builds lack them); the arbiter degrades to
            // "no visual z-order, just lifecycle + OOM" when missing.
            let layer_name = CString::new("sf_set_layer").unwrap();
            let layer_sym = libc::dlsym(handle, layer_name.as_ptr());
            let set_layer: Option<SetLayerFn> = if layer_sym.is_null() {
                None
            } else {
                Some(std::mem::transmute(layer_sym))
            };

            let visible_name = CString::new("sf_set_visible").unwrap();
            let visible_sym = libc::dlsym(handle, visible_name.as_ptr());
            let set_visible: Option<SetVisibleFn> = if visible_sym.is_null() {
                None
            } else {
                Some(std::mem::transmute(visible_sym))
            };

            let mut w: i32 = 0;
            let mut h: i32 = 0;
            let mut t: u32 = 0;
            let nw = create(&mut w, &mut h, &mut t);
            ensure!(!nw.is_null(), "sf_create_fullscreen_surface returned null");

            Ok(SfSurface {
                _handle: handle,
                native_window: nw,
                width: w,
                height: h,
                transform: t,
                input_poll,
                query_hint,
                request_focus,
                set_layer,
                set_visible,
            })
        }
    }

    /// Task 46 step 4/5 — reposition the wart layer on SurfaceFlinger's
    /// z-axis. Higher z is on top; `i32::MAX` is the default. Backgrounded
    /// apps should `set_layer(0)`. Returns `false` if the shim is too old
    /// to expose this (the arbiter then falls back to lifecycle + OOM
    /// without visual z-order).
    pub fn set_layer(&self, z: i32) -> bool {
        match self.set_layer {
            Some(f) => unsafe { f(z) == 0 },
            None    => false,
        }
    }

    /// Task 46 step 4/5 — toggle wart-layer visibility. Cheaper than
    /// re-allocating the surface for "background" — the layer stays
    /// alive, BBQ keeps the last frame, re-showing is one round-trip.
    pub fn set_visible(&self, visible: bool) -> bool {
        match self.set_visible {
            Some(f) => unsafe { f(if visible { 1 } else { 0 }) == 0 },
            None    => false,
        }
    }

    /// Query the live Android producer transform hint
    /// (`NATIVE_WINDOW_TRANSFORM_HINT`, a 0..7 bitmask). Call this only
    /// *after* EGL has connected the producer — the hint is not populated
    /// before that. Returns 0 if the shim predates this export.
    pub fn query_transform_hint(&self) -> u32 {
        self.query_hint.map(|f| unsafe { f() }).unwrap_or(0)
    }

    /// Re-request input focus for the wart window. Standalone has no Activity
    /// so activity-backed windows (launcher, last-resumed app) keep stealing
    /// focus from InputDispatcher's view — call this periodically (e.g. once
    /// per second) to keep keys flowing.
    pub fn request_focus(&self) {
        if let Some(f) = self.request_focus {
            unsafe { let _ = f(); }
        }
    }

    /// Drain pending input events from the shim into `buf`; returns the slice
    /// actually filled. Non-blocking — call once per frame.
    pub fn poll_input<'b>(&self, buf: &'b mut [SfInputEvent]) -> &'b [SfInputEvent] {
        let Some(poll) = self.input_poll else { return &[]; };
        let n = unsafe { poll(buf.as_mut_ptr(), buf.len() as i32) };
        let n = (n.max(0) as usize).min(buf.len());
        &buf[..n]
    }
}
