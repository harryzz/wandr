# Task 01 — Rust Host Skeleton

## Goal

`cargo build --target aarch64-linux-android` succeeds for the host crate.
The binary links against `libEGL.so` and `libandroid.so` from the NDK sysroot
and `skia-safe` with GL backend. No WASM loading yet — just the host compiles.

---

## Steps

### 1. Create the host crate

```bash
mkdir -p wasm-android-runtime/host/src
cd wasm-android-runtime
```

### 2. Create `host/Cargo.toml`

```toml
[package]
name    = "wasm-android-host"
version = "0.1.0"
edition = "2021"

# Build for Android: cargo build --target aarch64-linux-android --release
# Build for desktop: cargo build --release

[dependencies]
wasmtime           = { version = "27", features = ["component-model"], default-features = false }
wasmtime-wasi      = { version = "27", default-features = false }
anyhow             = "1"
log                = "1"
android_logger     = { version = "0.14", optional = true }
env_logger         = { version = "0.11", optional = true }
raw-window-handle  = "0.6"

# skia-safe: GL backend only (no Vulkan needed — EGL gives us GLES)
[dependencies.skia-safe]
version  = "0.75"
features = ["gl", "textlayout"]

# winit for window + event loop — android-native-activity on Android
[dependencies.winit]
version  = "0.30"
features = []

[target.'cfg(target_os = "android")'.dependencies]
winit            = { version = "0.30", features = ["android-native-activity"] }
android_logger   = "0.14"
ndk              = "0.9"
ndk-context      = "0.1"

[target.'cfg(not(target_os = "android"))'.dependencies]
env_logger = "0.11"
winit      = { version = "0.30", features = [] }

[build-dependencies]
# nothing yet — skia-safe brings its own build script

[features]
default = []
```

### 3. Create `host/build.rs`

```rust
fn main() {
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    if target == "android" {
        // Android NDK sysroot libraries
        // cargo sets ANDROID_NDK_HOME or we fall back to NDK_HOME
        let ndk = std::env::var("ANDROID_NDK_HOME")
            .or_else(|_| std::env::var("NDK_HOME"))
            .expect("ANDROID_NDK_HOME must be set when cross-compiling for Android");

        let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
        let abi = match arch.as_str() {
            "aarch64" => "arm64-v8a",
            "x86_64"  => "x86_64",
            other     => panic!("unsupported Android arch: {other}"),
        };

        // sysroot lib path for the target ABI
        // NDK r23+ layout
        let sysroot_lib = format!(
            "{ndk}/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/aarch64-linux-android"
        );

        println!("cargo:rustc-link-search={sysroot_lib}");
        println!("cargo:rustc-link-lib=EGL");
        println!("cargo:rustc-link-lib=android");
        println!("cargo:rustc-link-lib=log");
        println!("cargo:rustc-link-lib=GLESv2");
    }

    println!("cargo:rerun-if-changed=build.rs");
}
```

### 4. Create `host/src/main.rs` (stub — no WASM yet)

```rust
// host/src/main.rs
// Stub: just initialises logging and opens a window to verify
// the compile chain works. WASM loading added in Task 02.

mod egl;
mod canvas_impl;
mod input;

use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};
use std::sync::Arc;

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<canvas_impl::SkiaRenderer>,
}

impl App {
    fn new() -> Self {
        Self { window: None, renderer: None }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        log::info!("resumed — creating window");
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes()
                    .with_title("WASM Android Runtime"))
                .expect("window creation failed"),
        );

        // Initialise EGL and skia renderer once the native window is available
        let renderer = canvas_impl::SkiaRenderer::new(window.clone())
            .expect("renderer init failed");

        self.renderer = Some(renderer);
        self.window   = Some(window);
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("suspended — dropping renderer");
        self.renderer = None;  // release EGL surface
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
                if let Some(r) = &mut self.renderer {
                    r.draw_test_frame();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    android_logger::init_once(
        android_logger::Config::default().with_min_level(log::Level::Debug),
    );
    log::info!("android_main start");
    let event_loop = EventLoop::builder()
        .with_android_app(app)
        .build()
        .unwrap();
    event_loop.run_app(&mut App::new()).unwrap();
}

#[cfg(not(target_os = "android"))]
fn main() {
    env_logger::init();
    log::info!("desktop start");
    let event_loop = EventLoop::new().unwrap();
    event_loop.run_app(&mut App::new()).unwrap();
}
```

### 5. Create `host/src/egl.rs`

```rust
// host/src/egl.rs
// EGL context management for Android.
// On desktop this is a no-op stub — skia-safe uses its own GL context
// via glutin or similar, which is handled by canvas_impl.rs desktop path.

#[cfg(target_os = "android")]
pub mod android {
    use anyhow::{bail, Result};
    use std::ffi::c_void;

    // EGL types — we link libEGL.so directly
    type EGLDisplay  = *mut c_void;
    type EGLSurface  = *mut c_void;
    type EGLContext  = *mut c_void;
    type EGLConfig   = *mut c_void;
    type EGLint      = i32;
    type EGLBoolean  = u32;
    type EGLNativeWindowType = *mut c_void;

    const EGL_NONE:             EGLint = 0x3038;
    const EGL_SURFACE_TYPE:     EGLint = 0x3033;
    const EGL_WINDOW_BIT:       EGLint = 0x0004;
    const EGL_RENDERABLE_TYPE:  EGLint = 0x3040;
    const EGL_OPENGL_ES2_BIT:   EGLint = 0x0004;
    const EGL_BLUE_SIZE:        EGLint = 0x3022;
    const EGL_GREEN_SIZE:       EGLint = 0x3023;
    const EGL_RED_SIZE:         EGLint = 0x3024;
    const EGL_DEPTH_SIZE:       EGLint = 0x3025;
    const EGL_CONTEXT_CLIENT_VERSION: EGLint = 0x3098;
    const EGL_DEFAULT_DISPLAY:  EGLNativeWindowType = std::ptr::null_mut();
    const EGL_NO_DISPLAY:       EGLDisplay = std::ptr::null_mut();
    const EGL_NO_CONTEXT:       EGLContext = std::ptr::null_mut();
    const EGL_NO_SURFACE:       EGLSurface = std::ptr::null_mut();
    const EGL_TRUE:             EGLBoolean = 1;

    #[link(name = "EGL")]
    extern "C" {
        fn eglGetDisplay(display_id: EGLNativeWindowType) -> EGLDisplay;
        fn eglInitialize(dpy: EGLDisplay, major: *mut EGLint, minor: *mut EGLint) -> EGLBoolean;
        fn eglChooseConfig(dpy: EGLDisplay, attribs: *const EGLint,
                           configs: *mut EGLConfig, config_size: EGLint,
                           num_config: *mut EGLint) -> EGLBoolean;
        fn eglCreateWindowSurface(dpy: EGLDisplay, config: EGLConfig,
                                   win: EGLNativeWindowType,
                                   attribs: *const EGLint) -> EGLSurface;
        fn eglCreateContext(dpy: EGLDisplay, config: EGLConfig,
                            share: EGLContext,
                            attribs: *const EGLint) -> EGLContext;
        fn eglMakeCurrent(dpy: EGLDisplay, draw: EGLSurface,
                          read: EGLSurface, ctx: EGLContext) -> EGLBoolean;
        fn eglSwapBuffers(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean;
        fn eglDestroySurface(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean;
        fn eglDestroyContext(dpy: EGLDisplay, ctx: EGLContext) -> EGLBoolean;
        fn eglTerminate(dpy: EGLDisplay) -> EGLBoolean;
        fn eglGetProcAddress(name: *const u8) -> *const c_void;
    }

    pub struct EglContext {
        pub display: EGLDisplay,
        pub surface: EGLSurface,
        pub context: EGLContext,
        pub width:   i32,
        pub height:  i32,
    }

    impl EglContext {
        pub fn new(native_window: *mut c_void) -> Result<Self> {
            unsafe {
                let display = eglGetDisplay(EGL_DEFAULT_DISPLAY);
                if display == EGL_NO_DISPLAY { bail!("eglGetDisplay failed"); }

                let mut major = 0i32;
                let mut minor = 0i32;
                if eglInitialize(display, &mut major, &mut minor) != EGL_TRUE {
                    bail!("eglInitialize failed");
                }
                log::info!("EGL {major}.{minor}");

                let attribs = [
                    EGL_SURFACE_TYPE,    EGL_WINDOW_BIT,
                    EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
                    EGL_RED_SIZE,   8,
                    EGL_GREEN_SIZE, 8,
                    EGL_BLUE_SIZE,  8,
                    EGL_DEPTH_SIZE, 0,
                    EGL_NONE,
                ];
                let mut config: EGLConfig = std::ptr::null_mut();
                let mut num_config = 0i32;
                if eglChooseConfig(display, attribs.as_ptr(),
                                   &mut config, 1, &mut num_config) != EGL_TRUE
                    || num_config == 0
                {
                    bail!("eglChooseConfig failed");
                }

                let surface = eglCreateWindowSurface(
                    display, config, native_window, std::ptr::null());
                if surface == EGL_NO_SURFACE { bail!("eglCreateWindowSurface failed"); }

                let ctx_attribs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
                let context = eglCreateContext(
                    display, config, EGL_NO_CONTEXT, ctx_attribs.as_ptr());
                if context == EGL_NO_CONTEXT { bail!("eglCreateContext failed"); }

                eglMakeCurrent(display, surface, surface, context);

                // Query surface size
                let mut w = 0i32; let mut h = 0i32;
                // eglQuerySurface for width/height
                extern "C" {
                    fn eglQuerySurface(dpy: EGLDisplay, surface: EGLSurface,
                                       attr: EGLint, val: *mut EGLint) -> EGLBoolean;
                }
                eglQuerySurface(display, surface, 0x3056 /* EGL_WIDTH  */, &mut w);
                eglQuerySurface(display, surface, 0x3057 /* EGL_HEIGHT */, &mut h);

                Ok(EglContext { display, surface, context, width: w, height: h })
            }
        }

        pub fn swap(&self) {
            unsafe { eglSwapBuffers(self.display, self.surface); }
        }

        /// Return the eglGetProcAddress resolver for skia-safe's GL interface.
        pub fn proc_resolver() -> impl Fn(&str) -> *const c_void {
            |name: &str| {
                let c = std::ffi::CString::new(name).unwrap();
                unsafe { eglGetProcAddress(c.as_ptr() as *const u8) }
            }
        }
    }

    impl Drop for EglContext {
        fn drop(&mut self) {
            unsafe {
                eglMakeCurrent(
                    self.display,
                    EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
                eglDestroySurface(self.display, self.surface);
                eglDestroyContext(self.display, self.context);
                eglTerminate(self.display);
            }
        }
    }
}
```

### 6. Create `host/src/canvas_impl.rs` (stub renderer)

```rust
// host/src/canvas_impl.rs
// Skia renderer — draws a test frame.
// Full WIT canvas implementation added in Task 02.

use anyhow::Result;
use std::sync::Arc;
use winit::window::Window;

pub struct SkiaRenderer {
    #[cfg(target_os = "android")]
    egl:     crate::egl::android::EglContext,
    surface: skia_safe::Surface,
    width:   u32,
    height:  u32,
}

impl SkiaRenderer {
    pub fn new(window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();

        #[cfg(target_os = "android")]
        {
            // Get ANativeWindow pointer from winit
            use winit::platform::android::WindowExtAndroid;
            let native_window = window.a_native_window() as *mut std::ffi::c_void;
            let egl = crate::egl::android::EglContext::new(native_window)?;

            // Build skia-safe GL interface from EGL proc resolver
            let gl_interface = skia_safe::gpu::gl::Interface::new_load_with(
                crate::egl::android::EglContext::proc_resolver()
            ).ok_or_else(|| anyhow::anyhow!("GL interface failed"))?;

            let mut gr_context = skia_safe::gpu::direct_contexts::make_gl(
                gl_interface, None
            ).ok_or_else(|| anyhow::anyhow!("GrContext failed"))?;

            let fb_info = skia_safe::gpu::gl::FramebufferInfo {
                fboid:  0,
                format: skia_safe::gpu::gl::Format::RGBA8.into(),
                ..Default::default()
            };
            let backend_render_target = skia_safe::gpu::backend_render_targets::make_gl(
                (egl.width, egl.height), None, 8, fb_info
            );
            let surface = skia_safe::gpu::surfaces::wrap_backend_render_target(
                &mut gr_context,
                &backend_render_target,
                skia_safe::gpu::SurfaceOrigin::BottomLeft,
                skia_safe::ColorType::RGBA8888,
                None, None,
            ).ok_or_else(|| anyhow::anyhow!("Skia surface failed"))?;

            return Ok(SkiaRenderer {
                egl,
                surface,
                width:  size.width,
                height: size.height,
            });
        }

        #[cfg(not(target_os = "android"))]
        {
            // Desktop: use raster surface for now
            // Replace with GL/Vulkan surface for production desktop use
            let surface = skia_safe::surfaces::raster_n32_premul(
                (size.width as i32, size.height as i32)
            ).ok_or_else(|| anyhow::anyhow!("raster surface failed"))?;
            Ok(SkiaRenderer { surface, width: size.width, height: size.height })
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.width  = w;
        self.height = h;
        // TODO: recreate surface at new size (Task 02)
    }

    pub fn draw_test_frame(&mut self) {
        let canvas = self.surface.canvas();

        // Clear to dark blue
        canvas.clear(skia_safe::Color::from_argb(255, 10, 20, 60));

        // White rectangle
        canvas.draw_rect(
            skia_safe::Rect::from_xywh(
                self.width  as f32 * 0.1,
                self.height as f32 * 0.1,
                self.width  as f32 * 0.8,
                self.height as f32 * 0.3,
            ),
            &skia_safe::Paint::new(skia_safe::Color4f::new(1.0, 1.0, 1.0, 0.9), None),
        );

        self.surface.flush_and_submit();

        #[cfg(target_os = "android")]
        self.egl.swap();
    }
}

// input.rs stub — filled in Task 05
```

### 7. Create `host/src/input.rs` (empty stub)

```rust
// host/src/input.rs
// Input event dispatch — implemented in Task 05.
pub struct InputDispatcher;
```

### 8. Create `.cargo/config.toml` for Android cross-compilation

```bash
mkdir -p wasm-android-runtime/.cargo
```

File `.cargo/config.toml`:

```toml
[target.aarch64-linux-android]
linker = "aarch64-linux-android35-clang"

# Set ANDROID_NDK_HOME and ensure the NDK clang is in PATH:
# export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
```

### 9. Add Android target to Rust

```bash
rustup target add aarch64-linux-android
```

---

## Verify

### Desktop build (sanity check first)

```bash
cd wasm-android-runtime/host
cargo build 2>&1 | tail -5
# Expected: "Finished dev" — no errors
```

### Android cross-compile

```bash
export ANDROID_NDK_HOME=/path/to/your/ndk   # set this
export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"

cargo build --target aarch64-linux-android 2>&1 | tail -10
# Expected: "Finished dev" — libwasm_android_host.so or binary created
```

Check the binary exists:
```bash
ls -lh target/aarch64-linux-android/debug/wasm-android-host
# Expected: file exists, ~10–50MB (includes skia)
```

Check it links EGL:
```bash
$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf \
  -d target/aarch64-linux-android/debug/wasm-android-host \
  | grep EGL
# Expected: libEGL.so appears in NEEDED entries
```

### ✅ Checkpoint — write this after all checks pass

```bash
cat > wasm-android-runtime/.task-state << 'EOF'
TASK=01
STEP=verify-done
STATUS=complete
LAST_SUCCESS=Task 01 verified OK — host compiles for desktop and aarch64-linux-android, EGL linked
NOTES=
EOF
```

---

## Known issues

### `skia-safe` build fails with missing `clang`

skia-safe builds Skia from source using LLVM. Set:
```bash
export CC_aarch64_linux_android=aarch64-linux-android35-clang
export CXX_aarch64_linux_android=aarch64-linux-android35-clang++
export AR_aarch64_linux_android=llvm-ar
```

### `pwritev64` undefined reference

Old NDK (< r23). Upgrade to NDK r26+. The issue was present in NDK r21 and
fixed in later versions.

### `stderr` undefined reference from skia's zstd

Same old NDK issue. NDK r26+ fixes this.

### `eglGetProcAddress` returns null for some functions on Android

On some Android versions, `eglGetProcAddress` doesn't return core GL functions.
Add a fallback:
```rust
fn gl_get_proc_address(name: &str) -> *const c_void {
    let ptr = egl_get_proc_address(name);
    if ptr.is_null() {
        // try dlsym from libGLESv2.so directly
        dlsym_gles2(name)
    } else {
        ptr
    }
}
```

### winit `a_native_window()` not found

Ensure `winit` dep has `features = ["android-native-activity"]` under
the `[target.'cfg(target_os = "android")'.dependencies]` section specifically.
The feature must be platform-gated or it conflicts with desktop builds.

## Do NOT

- Do not add `wgpu` as a dependency. We are using EGL + skia-safe GL directly.
- Do not try to run the Android binary on desktop or vice versa.
- Do not skip the desktop build check — it catches Rust errors faster.
