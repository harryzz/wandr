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

/// `ANativeWindow* sf_create_fullscreen_surface(int32_t*, int32_t*)`.
type CreateFn = unsafe extern "C" fn(*mut i32, *mut i32) -> *mut c_void;

/// A fullscreen surface allocated from SurfaceFlinger by `libsf_surface.so`.
/// Keeps the `dlopen` handle for the process lifetime — unloading the shim
/// would invalidate `native_window`.
pub struct SfSurface {
    _handle: *mut c_void,
    /// `ANativeWindow*` — hand straight to `EglContext::new`.
    pub native_window: *mut c_void,
    pub width: i32,
    pub height: i32,
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

            let mut w: i32 = 0;
            let mut h: i32 = 0;
            let nw = create(&mut w, &mut h);
            ensure!(!nw.is_null(), "sf_create_fullscreen_surface returned null");

            Ok(SfSurface { _handle: handle, native_window: nw, width: w, height: h })
        }
    }
}
