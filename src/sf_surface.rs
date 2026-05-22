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

/// POD input event drained from the shim's InputFlinger channel. Mirrors
/// `struct SfInputEvent` in `cpp/sf_surface.{cpp,h}` — keep all three in sync.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SfInputEvent {
    /// 0=down 1=up 2=move 3=scroll.
    pub kind: i32,
    /// Multi-touch pointer id (0..N).
    pub pointer_id: i32,
    pub x: f32,
    pub y: f32,
    /// Normalized pressure 0.0..1.0.
    pub pressure: f32,
    /// Reserved — key events are not emitted in this cut.
    pub key_code: i32,
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
            })
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
