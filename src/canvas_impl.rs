use anyhow::Result;
use skia_safe::{Canvas, Color, Font, Paint, PaintStyle, Rect, RRect, Surface, Typeface};
use std::collections::HashMap;
use std::sync::Arc;
use winit::window::Window;

use crate::bindings::my::skiko_gfx::canvas::{
    BlendMode, ColorFilterKind, PaintAttrs, PaintStyle as WitPaintStyle, StrokeCap, StrokeJoin,
};

// ─── Rasterized-text cache ───────────────────────────────────────────────────
//
// Without this cache `blit_text_blob` allocates a fresh CPU surface +
// `SkImage` on every call and Skia uploads each as a unique GPU texture that
// is never reused. With ~50 text draws per frame at 60 fps that's ~3000
// texture uploads/sec and ~9 MB/sec leak. Caching by (blob-bounds-hash,
// paint colour) caps the working set at O(distinct labels).

struct CachedTextImage {
    image:    skia_safe::Image,
    offset_x: f32,
    offset_y: f32,
}

const TEXT_IMAGE_CACHE_CAP: usize = 256;

fn rasterize_text_blob(blob: &skia_safe::TextBlob, paint: &Paint) -> Option<CachedTextImage> {
    let bounds = blob.bounds();
    let img_w = (bounds.width().ceil()  as i32 + 4).max(1);
    let img_h = (bounds.height().ceil() as i32 + 4).max(1);
    let mut cpu = skia_safe::surfaces::raster_n32_premul((img_w, img_h))?;
    cpu.canvas().clear(Color::TRANSPARENT);
    cpu.canvas().draw_text_blob(blob, (-bounds.left() + 1.0, -bounds.top() + 1.0), paint);
    Some(CachedTextImage {
        image:    cpu.image_snapshot(),
        offset_x: bounds.left() - 1.0,
        offset_y: bounds.top() - 1.0,
    })
}

fn paint_cache_key(p: &Paint) -> u32 {
    // skia_safe::Color is repr(transparent) wrapping SkColor (u32) —
    // safe to transmute.
    unsafe { std::mem::transmute::<skia_safe::Color, u32>(p.color()) }
}

/// Content-based hash of a text blob: text + font params. Two blobs with the
/// same content hash render identically; two with different content always
/// get different keys (regardless of whether their visual bounds match).
fn text_content_hash(text: &str, family: &str, size: f32, weight: u32, italic: bool) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    family.hash(&mut h);
    size.to_bits().hash(&mut h);
    weight.hash(&mut h);
    italic.hash(&mut h);
    h.finish()
}

// ─── WasiDrawable FFI ────────────────────────────────────────────────────────
//
// C++ shim in host/cpp/wasi_drawable.{h,cpp} subclasses SkDrawable with a
// mutable picture handle so parent recordings can capture `drawDrawable(id)`
// ops that resolve to the CURRENT picture at replay time. See
// cpp/wasi_drawable.h for rationale.

mod wasi_drawable_ffi {
    use std::os::raw::c_void;
    extern "C" {
        pub fn wasi_drawable_create() -> *mut c_void;
        pub fn wasi_drawable_set_inner(outer: *mut c_void, inner: *mut c_void);
        pub fn wasi_drawable_set_bounds(d: *mut c_void,
                                        l: f32, t: f32, r: f32, b: f32);
        pub fn wasi_drawable_set_transform(d: *mut c_void,
                                           layer_x: f32, layer_y: f32,
                                           translation_x: f32, translation_y: f32,
                                           scale_x: f32, scale_y: f32,
                                           rotation_z: f32,
                                           pivot_x: f32, pivot_y: f32,
                                           alpha: f32);
        pub fn wasi_drawable_set_clip_rect(d: *mut c_void,
                                           l: f32, t: f32, r: f32, b: f32,
                                           antialias: bool);
        pub fn wasi_drawable_set_clip_rrect(d: *mut c_void,
                                            l: f32, t: f32, r: f32, b: f32,
                                            radii_xy_4_corners: *const f32,
                                            antialias: bool);
        pub fn wasi_drawable_clear_clip(d: *mut c_void);
        pub fn wasi_drawable_set_shadow_elevation(d: *mut c_void, elevation: f32);
        pub fn wasi_drawable_ref(d: *mut c_void);
        pub fn wasi_drawable_unref(d: *mut c_void);
        pub fn wasi_canvas_draw_drawable(canvas: *mut c_void, d: *mut c_void);
    }
}

/// Read the underlying raw `SkPicture*` (or `SkCanvas*`, `SkDrawable*`, …)
/// out of a skia-safe handle. `RCHandle<N>` and `RefHandle<N>` are both
/// single-field tuple structs over `ptr::NonNull<N>`, so the first 8 bytes
/// of the struct are the native pointer. `NonNull` is `#[repr(transparent)]`
/// over `*const N`, and a single-field tuple struct over a transparent
/// type has the same starting layout. We use this to bridge skia-safe ↔
/// our C FFI without going through `pub(crate)` `NativeAccess`/`from_ptr`.
#[inline]
fn handle_to_native_ptr<T>(handle: *const T) -> *mut std::os::raw::c_void {
    unsafe { *(handle as *const *mut std::os::raw::c_void) }
}

/// Owned handle to a WasiDrawable. Holds one ref; Drop releases it.
pub struct WasiDrawable {
    ptr: *mut std::os::raw::c_void,
}

impl WasiDrawable {
    pub fn new() -> Self {
        Self { ptr: unsafe { wasi_drawable_ffi::wasi_drawable_create() } }
    }

    /// Swap the inner SkDrawable this wrapper delegates to. `None` clears it.
    pub fn set_inner(&mut self, inner: Option<&skia_safe::Drawable>) {
        let inner_ptr = match inner {
            Some(d) => handle_to_native_ptr(d as *const skia_safe::Drawable),
            None    => std::ptr::null_mut(),
        };
        unsafe { wasi_drawable_ffi::wasi_drawable_set_inner(self.ptr, inner_ptr); }
    }

    pub fn set_bounds(&mut self, l: f32, t: f32, r: f32, b: f32) {
        unsafe { wasi_drawable_ffi::wasi_drawable_set_bounds(self.ptr, l, t, r, b); }
    }

    pub fn as_ptr(&self) -> *mut std::os::raw::c_void { self.ptr }
}

impl Drop for WasiDrawable {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { wasi_drawable_ffi::wasi_drawable_unref(self.ptr); }
        }
    }
}

// SkDrawable refcount is non-atomic but the renderer is never shared across
// threads (winit event-loop only), so this is sound — matches the unsafe
// impl Send for SkiaRenderer below.
unsafe impl Send for WasiDrawable {}

// ─── Multi-run text-blob builder ─────────────────────────────────────────────

struct TextBlobRun {
    text:   String,
    family: String,
    size:   f32,
    weight: u32,
    italic: bool,
    x:      f32,
    y:      f32,
}

// ─── Renderer state ──────────────────────────────────────────────────────────

pub struct SkiaRenderer {
    // Drop order matters: gr_context + surface must drop before egl so that
    // Skia's GL cleanup happens while the EGL context is still bound.
    #[cfg(target_os = "android")]
    gr_context: skia_safe::gpu::DirectContext,

    surface:    Surface,
    pub width:  u32,
    pub height: u32,

    #[cfg(target_os = "android")]
    egl:        crate::egl::android::EglContext,

    // Each blob carries a content hash (text + font params) so the text-image
    // cache key can distinguish "Count: 5" from "Count: 0" — same bounds,
    // different content. Without this the cache returns a stale GPU texture
    // and the displayed text never updates.
    text_blobs:       HashMap<u32, (skia_safe::TextBlob, u64)>,
    multi_blob_cache: HashMap<u32, Vec<(skia_safe::TextBlob, f32, f32, u64)>>,
    text_blob_runs:   Vec<TextBlobRun>,
    images:           HashMap<u32, skia_safe::Image>,
    shader_cache:     HashMap<u32, skia_safe::Shader>,
    next_blob_id:     u32,
    next_shader_id:   u32,
    // Picture recording (Tier A skia shim). recorders are in either
    // "idle" or "recording" state; recording_stack holds the IDs of
    // recorders currently in begin_recording → finish state, with the
    // top redirecting `canvas()` draws into the recorder's canvas.
    recorders:        HashMap<u32, skia_safe::PictureRecorder>,
    pictures:         HashMap<u32, skia_safe::Picture>,
    recording_stack:  Vec<u32>,
    next_recorder_id: u32,
    next_picture_id:  u32,
    // WasiDrawable instances (deferred-replay shim). Each maps id → owned
    // SkDrawable*. Parent recordings hold raw pointers via drawDrawable, so
    // dropping a drawable while a parent picture still references it would
    // dangling. Compose drops them at RenderNode.close() AFTER releasing
    // the parent layer that referenced them, which is correct order.
    drawables:        HashMap<u32, WasiDrawable>,
    next_drawable_id: u32,
    typeface_cache:   HashMap<(String, bool, bool), Typeface>,

    text_image_cache: HashMap<(u64, u32), CachedTextImage>,
    text_image_keys:  std::collections::VecDeque<(u64, u32)>,

    pub para_builders:   HashMap<u32, skia_safe::textlayout::ParagraphBuilder>,
    pub paragraphs:      HashMap<u32, skia_safe::textlayout::Paragraph>,
    pub font_collection: skia_safe::textlayout::FontCollection,
    pub next_para_id:    u32,
    /// Holds the result of the last `prepare-rects-for-range` call so the
    /// guest can pull rect fields out via indexed getters (avoiding the
    /// need for `list<f32>` return marshaling in the WIT bindings). One
    /// renderer-wide slot is sufficient: the guest always reads the cache
    /// in the same WIT call burst, never interleaved with another prepare.
    pub para_rect_cache: Vec<skia_safe::textlayout::TextBox>,
}

// Skia's RCHandle uses non-atomic refcounts so its types aren't auto-Send.
// We hold the renderer in a wasmtime Store whose `T: WasiView: Send` bound
// forces HostState to be Send. The renderer is never shared across threads —
// the entire host runs on the winit event-loop thread — so this is sound.
unsafe impl Send for SkiaRenderer {}

impl SkiaRenderer {
    pub fn new(window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();

        #[cfg(target_os = "android")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let raw = window
                .window_handle()
                .map_err(|e| anyhow::anyhow!("window_handle failed: {e:?}"))?
                .as_raw();
            let native_window = match raw {
                RawWindowHandle::AndroidNdk(h) => h.a_native_window.as_ptr(),
                other => return Err(anyhow::anyhow!(
                    "expected AndroidNdk window handle, got {other:?}"
                )),
            };
            let egl = crate::egl::android::EglContext::new(native_window)?;

            let gl_interface = skia_safe::gpu::gl::Interface::new_load_with(
                crate::egl::android::EglContext::proc_resolver()
            ).ok_or_else(|| anyhow::anyhow!("GL interface failed"))?;

            let mut gr_context = skia_safe::gpu::direct_contexts::make_gl(
                gl_interface, None
            ).ok_or_else(|| anyhow::anyhow!("GrContext failed"))?;

            let surface = Self::make_gl_surface(
                &mut gr_context, egl.width, egl.height)?;

            return Ok(Self {
                egl, gr_context, surface,
                width: size.width, height: size.height,
                text_blobs:       HashMap::new(),
                multi_blob_cache: HashMap::new(),
                text_blob_runs:   Vec::new(),
                images:           HashMap::new(),
                shader_cache:     HashMap::new(),
                next_blob_id:     1,
                next_shader_id:   1,
                recorders:        HashMap::new(),
                pictures:         HashMap::new(),
                recording_stack:  Vec::new(),
                next_recorder_id: 1,
                next_picture_id:  1,
                drawables:        HashMap::new(),
                next_drawable_id: 1,
                typeface_cache:   HashMap::new(),
                text_image_cache: HashMap::new(),
                text_image_keys:  std::collections::VecDeque::with_capacity(TEXT_IMAGE_CACHE_CAP),
                para_builders:    HashMap::new(),
                paragraphs:       HashMap::new(),
                font_collection:  Self::make_font_collection(),
                next_para_id:     1,
                para_rect_cache:  Vec::new(),
            });
        }

        #[cfg(not(target_os = "android"))]
        {
            let surface = skia_safe::surfaces::raster_n32_premul(
                (size.width as i32, size.height as i32)
            ).ok_or_else(|| anyhow::anyhow!("raster surface failed"))?;
            Ok(Self {
                surface, width: size.width, height: size.height,
                text_blobs:       HashMap::new(),
                multi_blob_cache: HashMap::new(),
                text_blob_runs:   Vec::new(),
                images:           HashMap::new(),
                shader_cache:     HashMap::new(),
                next_blob_id:     1,
                next_shader_id:   1,
                recorders:        HashMap::new(),
                pictures:         HashMap::new(),
                recording_stack:  Vec::new(),
                next_recorder_id: 1,
                next_picture_id:  1,
                drawables:        HashMap::new(),
                next_drawable_id: 1,
                typeface_cache:   HashMap::new(),
                text_image_cache: HashMap::new(),
                text_image_keys:  std::collections::VecDeque::with_capacity(TEXT_IMAGE_CACHE_CAP),
                para_builders:    HashMap::new(),
                paragraphs:       HashMap::new(),
                font_collection:  Self::make_font_collection(),
                next_para_id:     1,
                para_rect_cache:  Vec::new(),
            })
        }
    }

    /// Move CPU-side caches from `old` into `self` so warm-resume preserves
    /// wasm-allocated handle IDs (pictures, recorders, text blobs, shaders,
    /// paragraphs, ...). The next_*_id counters carry over so the next ID
    /// the guest mints doesn't collide with one already in the inherited
    /// tables. GPU-resident caches (`text_image_cache`, `images`) are NOT
    /// inherited because their textures live in the dying gr_context.
    pub fn inherit_caches_from(&mut self, old: &mut Self) {
        self.text_blobs       = std::mem::take(&mut old.text_blobs);
        self.multi_blob_cache = std::mem::take(&mut old.multi_blob_cache);
        self.text_blob_runs   = std::mem::take(&mut old.text_blob_runs);
        self.shader_cache     = std::mem::take(&mut old.shader_cache);
        self.next_blob_id     = old.next_blob_id;
        self.next_shader_id   = old.next_shader_id;
        self.recorders        = std::mem::take(&mut old.recorders);
        self.pictures         = std::mem::take(&mut old.pictures);
        self.recording_stack  = std::mem::take(&mut old.recording_stack);
        self.next_recorder_id = old.next_recorder_id;
        self.next_picture_id  = old.next_picture_id;
        self.drawables        = std::mem::take(&mut old.drawables);
        self.next_drawable_id = old.next_drawable_id;
        self.typeface_cache   = std::mem::take(&mut old.typeface_cache);
        self.para_builders    = std::mem::take(&mut old.para_builders);
        self.paragraphs       = std::mem::take(&mut old.paragraphs);
        self.next_para_id     = old.next_para_id;
        // font_collection holds a default-FontMgr; keep the freshly built
        // one to be safe (cheap to recreate).
    }

    #[cfg(target_os = "android")]
    fn make_gl_surface(
        gr: &mut skia_safe::gpu::DirectContext,
        w: i32, h: i32,
    ) -> Result<Surface> {
        let fb_info = skia_safe::gpu::gl::FramebufferInfo {
            fboid:     0,
            format:    skia_safe::gpu::gl::Format::RGBA8.into(),
            protected: skia_safe::gpu::Protected::No,
        };
        let target = skia_safe::gpu::backend_render_targets::make_gl(
            (w, h), Some(0), 8, fb_info);
        skia_safe::gpu::surfaces::wrap_backend_render_target(
            gr, &target,
            skia_safe::gpu::SurfaceOrigin::BottomLeft,
            skia_safe::ColorType::RGBA8888,
            None, None,
        ).ok_or_else(|| anyhow::anyhow!("wrap_backend_render_target failed"))
    }

    fn make_font_collection() -> skia_safe::textlayout::FontCollection {
        let mut fc = skia_safe::textlayout::FontCollection::new();
        let mgr = skia_safe::FontMgr::new();
        fc.set_default_font_manager(mgr, None);
        fc
    }

    pub fn canvas(&mut self) -> &Canvas {
        // If a picture recording is active, route draw calls into the
        // recorder's canvas instead of the screen surface. The recorder owns
        // an internal Canvas during begin_recording → finish; we look it up
        // by the top-of-stack recorder id.
        if let Some(&rid) = self.recording_stack.last() {
            if let Some(rec) = self.recorders.get_mut(&rid) {
                if let Some(c) = rec.recording_canvas() {
                    // Lifetime extension: skia-safe returns &Canvas borrowed
                    // from `self` through the recorder; that's the same
                    // shape callers expect from `surface.canvas()`. Safe so
                    // long as callers don't hold the borrow across another
                    // `&mut self` call (mirrors the surface.canvas() rules).
                    return unsafe { &*(c as *const skia_safe::Canvas) };
                }
            }
        }
        self.surface.canvas()
    }

    pub fn flush_and_swap(&mut self) {
        #[cfg(target_os = "android")]
        {
            self.egl.make_current();
            self.gr_context.flush_and_submit();
            self.egl.swap();
            // Each `blit_text_blob_cached` miss uploads a CPU raster to a GPU
            // texture. The cached SkImage holds a reference to that texture
            // for next-frame reuse. Without this purge, Skia's resource cache
            // ALSO retains scratch/throwaway resources from path tessellation,
            // gradient shaders, etc. — capping ~9 MB/sec growth on the showcase.
            self.gr_context.purge_unlocked_resources(
                skia_safe::gpu::PurgeResourceOptions::AllResources,
            );
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.width  = w;
        self.height = h;
        #[cfg(target_os = "android")]
        {
            if let Ok(s) = Self::make_gl_surface(
                &mut self.gr_context, w as i32, h as i32)
            {
                self.surface = s;
            }
        }
        #[cfg(not(target_os = "android"))]
        {
            if let Some(s) = skia_safe::surfaces::raster_n32_premul(
                (w as i32, h as i32)) {
                self.surface = s;
            }
        }
    }

    pub fn draw_test_frame(&mut self) {
        #[cfg(target_os = "android")]
        self.egl.make_current();
        {
            let c = self.surface.canvas();
            c.clear(Color::from_argb(255, 10, 20, 60));
            c.draw_rect(
                Rect::from_xywh(50.0, 50.0, 200.0, 100.0),
                &Paint::new(skia_safe::Color4f::new(1.0, 1.0, 1.0, 1.0), None),
            );
        }
        self.flush_and_swap();
    }

    /// Returns a Typeface for the requested (family, bold, italic), reading
    /// from /system/fonts and caching the result.
    pub fn get_typeface(&mut self, family: &str, bold: bool, italic: bool) -> Typeface {
        let key = (family.to_string(), bold, italic);
        if let Some(tf) = self.typeface_cache.get(&key) {
            return tf.clone();
        }
        // If the family is an absolute path, try that first.
        let mut candidates: Vec<String> = Vec::new();
        if family.starts_with('/') {
            candidates.push(family.to_string());
        }
        // Match-family-style on Skia's default FontMgr gives zero-metrics
        // typefaces on this device, so we always load from a TTF file.
        candidates.extend(font_candidate_paths(bold, italic).iter().map(|s| s.to_string()));
        let mgr = skia_safe::FontMgr::new();
        for path in &candidates {
            if let Ok(bytes) = std::fs::read(path) {
                if let Some(tf) = mgr.new_from_data(&bytes, None) {
                    self.typeface_cache.insert(key.clone(), tf.clone());
                    log::info!("get_typeface: loaded {path} (bold={bold} italic={italic})");
                    return tf;
                }
            }
        }
        // Last-ditch fallback — Skia's default empty typeface.
        let mgr = skia_safe::FontMgr::new();
        let tf = mgr.legacy_make_typeface(None, skia_safe::FontStyle::normal())
            .expect("no fallback typeface available from FontMgr");
        self.typeface_cache.insert(key, tf.clone());
        tf
    }

    /// CPU-rasterise the blob then blit to the GPU canvas, caching the
    /// SkImage so identical (blob content, paint colour) draws reuse the
    /// same GPU texture. The content hash is computed in `create_text_blob`
    /// from text + font params — distinct content always gets distinct keys
    /// even when bounds collide.
    fn blit_text_blob_cached(
        &mut self,
        blob: &skia_safe::TextBlob,
        content_hash: u64,
        x: f32, y: f32,
        paint: &Paint,
    ) {
        let key = (content_hash, paint_cache_key(paint));

        if !self.text_image_cache.contains_key(&key) {
            let entry = match rasterize_text_blob(blob, paint) {
                Some(e) => e,
                None    => return,
            };
            if self.text_image_keys.len() >= TEXT_IMAGE_CACHE_CAP {
                if let Some(old) = self.text_image_keys.pop_front() {
                    self.text_image_cache.remove(&old);
                }
            }
            self.text_image_cache.insert(key, entry);
            self.text_image_keys.push_back(key);
        }
        // Use canvas() helper so the image lands on the recording canvas
        // when a Picture is being recorded, not the screen surface.
        let image = self.text_image_cache.get(&key).unwrap().image.clone();
        let ox = self.text_image_cache.get(&key).unwrap().offset_x;
        let oy = self.text_image_cache.get(&key).unwrap().offset_y;
        self.canvas().draw_image(&image, (x + ox, y + oy), None);
    }

    pub fn draw_paragraph(&mut self, id: u32, x: f32, y: f32) {
        // Skia's Paragraph (RefHandle) isn't Clone. We need to paint via
        // self.canvas() which respects the recording stack. Hold the paragraph
        // and the recorder-or-surface canvas as raw pointers briefly so the
        // borrow checker doesn't see overlapping borrows. Safe because we
        // never re-enter `self` during the paint call.
        let para_ptr: *const skia_safe::textlayout::Paragraph =
            match self.paragraphs.get(&id) {
                Some(p) => p as *const _,
                None => return,
            };
        let canvas_ptr: *const Canvas = self.canvas() as *const Canvas;
        unsafe { (&*para_ptr).paint(&*canvas_ptr, (x, y)); }
    }
    #[allow(dead_code)]
    fn _old_draw_paragraph(&mut self, id: u32, x: f32, y: f32) {
        if let Some(p) = self.paragraphs.get(&id) {
            p.paint(self.surface.canvas(), (x, y));
        }
    }
}

fn font_candidate_paths(bold: bool, italic: bool) -> &'static [&'static str] {
    match (bold, italic) {
        (true,  true ) => &[
            "/system/fonts/Roboto-BoldItalic.ttf",
            "/system/fonts/SourceSansPro-BoldItalic.ttf",
            "/system/fonts/DroidSans-Bold.ttf",
        ],
        (true,  false) => &[
            "/system/fonts/Roboto-Bold.ttf",
            "/system/fonts/SourceSansPro-Bold.ttf",
            "/system/fonts/DroidSans-Bold.ttf",
        ],
        (false, true ) => &[
            "/system/fonts/Roboto-Italic.ttf",
            "/system/fonts/SourceSansPro-Italic.ttf",
            "/system/fonts/DroidSans.ttf",
        ],
        (false, false) => &[
            "/system/fonts/Roboto-Regular.ttf",
            "/system/fonts/SourceSansPro-Regular.ttf",
            "/system/fonts/DroidSans.ttf",
        ],
    }
}

// ─── WIT canvas trait implementation ─────────────────────────────────────────

impl crate::bindings::my::skiko_gfx::canvas::Host for crate::HostState {

    fn surface_width (&mut self) -> u32 { self.renderer.width  }
    fn surface_height(&mut self) -> u32 { self.renderer.height }

    fn begin_frame(&mut self) {
        #[cfg(target_os = "android")]
        {
            static LOGGED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::info!("begin_frame: size={}x{}",
                    self.renderer.width, self.renderer.height);
            }
            self.renderer.egl.make_current();
        }
    }

    fn end_frame(&mut self) {
        self.renderer.flush_and_swap();
    }

    fn save(&mut self)    { self.renderer.canvas().save(); }
    fn restore(&mut self) { self.renderer.canvas().restore(); }

    fn save_layer(&mut self, x: f32, y: f32, w: f32, h: f32, has_bounds: bool, alpha: u8) {
        if has_bounds {
            self.renderer.canvas()
                .save_layer_alpha(Some(Rect::from_xywh(x, y, w, h)), alpha as u32);
        } else {
            self.renderer.canvas().save_layer_alpha(None, alpha as u32);
        }
    }

    fn translate(&mut self, dx: f32, dy: f32) { self.renderer.canvas().translate((dx, dy)); }
    fn scale    (&mut self, sx: f32, sy: f32) { self.renderer.canvas().scale((sx, sy));     }
    fn rotate   (&mut self, deg: f32)         { self.renderer.canvas().rotate(deg, None);   }
    fn skew     (&mut self, sx: f32, sy: f32) { self.renderer.canvas().skew((sx, sy));      }

    fn concat(&mut self, a: f32, b: f32, c: f32,
                         d: f32, e: f32, f: f32,
                         g: f32, h: f32, i: f32) {
        let m = skia_safe::Matrix::new_all(a, b, c, d, e, f, g, h, i);
        self.renderer.canvas().concat(&m);
    }

    fn reset_matrix(&mut self) {
        self.renderer.canvas().reset_matrix();
    }

    fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32, anti_alias: bool) {
        self.renderer.canvas().clip_rect(
            Rect::from_xywh(x, y, w, h),
            Some(skia_safe::ClipOp::Intersect),
            Some(anti_alias),
        );
    }

    fn clip_rrect(&mut self, x: f32, y: f32, w: f32, h: f32,
                   rx: f32, ry: f32, anti_alias: bool) {
        let rr = RRect::new_rect_xy(Rect::from_xywh(x, y, w, h), rx, ry);
        self.renderer.canvas().clip_rrect(
            rr, Some(skia_safe::ClipOp::Intersect), Some(anti_alias),
        );
    }

    fn clip_path(&mut self, path_data: Vec<u8>, anti_alias: bool) {
        let s = String::from_utf8_lossy(&path_data);
        if let Some(p) = skia_safe::Path::from_svg(&*s) {
            self.renderer.canvas().clip_path(
                &p, Some(skia_safe::ClipOp::Intersect), Some(anti_alias),
            );
        }
    }

    fn clear(&mut self, argb: u32) {
        self.renderer.canvas().clear(Color::new(argb));
    }

    fn draw_paint(&mut self, p: PaintAttrs) {
        let paint = make_paint_full(&p, &self.renderer);
        self.renderer.canvas().draw_paint(&paint);
    }

    fn draw_rect(&mut self, x: f32, y: f32, w: f32, h: f32, p: PaintAttrs) {
        let paint = make_paint_full(&p, &self.renderer);
        self.renderer.canvas().draw_rect(Rect::from_xywh(x, y, w, h), &paint);
    }

    fn draw_rrect(&mut self, x: f32, y: f32, w: f32, h: f32,
                   rx: f32, ry: f32, p: PaintAttrs) {
        let paint = make_paint_full(&p, &self.renderer);
        let rr    = RRect::new_rect_xy(Rect::from_xywh(x, y, w, h), rx, ry);
        self.renderer.canvas().draw_rrect(rr, &paint);
    }

    fn draw_drrect(&mut self,
        ox: f32, oy: f32, ow: f32, oh: f32, orx: f32, ory: f32,
        ix: f32, iy: f32, iw: f32, ih: f32, irx: f32, iry: f32,
        p: PaintAttrs,
    ) {
        let paint = make_paint_full(&p, &self.renderer);
        let outer = RRect::new_rect_xy(Rect::from_xywh(ox, oy, ow, oh), orx, ory);
        let inner = RRect::new_rect_xy(Rect::from_xywh(ix, iy, iw, ih), irx, iry);
        self.renderer.canvas().draw_drrect(outer, inner, &paint);
    }

    fn draw_oval(&mut self, x: f32, y: f32, w: f32, h: f32, p: PaintAttrs) {
        let paint = make_paint_full(&p, &self.renderer);
        self.renderer.canvas().draw_oval(Rect::from_xywh(x, y, w, h), &paint);
    }

    fn draw_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, p: PaintAttrs) {
        let paint = make_paint_full(&p, &self.renderer);
        self.renderer.canvas().draw_line((x0, y0), (x1, y1), &paint);
    }

    fn draw_arc(&mut self, x: f32, y: f32, w: f32, h: f32,
                start_angle: f32, sweep_angle: f32, include_center: bool,
                p: PaintAttrs) {
        let paint = make_paint_full(&p, &self.renderer);
        self.renderer.canvas().draw_arc(
            Rect::from_xywh(x, y, w, h),
            start_angle, sweep_angle, include_center, &paint,
        );
    }

    fn draw_path(&mut self, path_data: Vec<u8>, p: PaintAttrs) {
        let paint = make_paint_full(&p, &self.renderer);
        let s = String::from_utf8_lossy(&path_data);
        if let Some(path) = skia_safe::Path::from_svg(&*s) {
            self.renderer.canvas().draw_path(&path, &paint);
        }
    }

    // ── text blobs ────────────────────────────────────────────────────────

    fn create_text_blob(&mut self, text: Vec<u8>, font_family: Vec<u8>,
                         size: f32, weight: u32, italic: bool) -> u32 {
        let text_str   = String::from_utf8_lossy(&text).into_owned();
        let family_str = String::from_utf8_lossy(&font_family).into_owned();
        let bold       = weight >= 600;
        let tf         = self.renderer.get_typeface(&family_str, bold, italic);
        let mut font   = Font::new(tf, size);
        font.set_edging(skia_safe::font::Edging::AntiAlias);
        font.set_subpixel(false);
        let blob = skia_safe::TextBlob::from_str(&text_str, &font);
        let content_hash = text_content_hash(&text_str, &family_str, size, weight, italic);
        static ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let b = blob.as_ref().map(|b| b.bounds());
            log::info!("create_text_blob first: size={size} blob={} bounds={b:?}",
                blob.is_some());
        }
        let id = self.renderer.next_blob_id;
        self.renderer.next_blob_id = id.wrapping_add(1).max(1);
        if let Some(b) = blob {
            self.renderer.text_blobs.insert(id, (b, content_hash));
        }
        id
    }

    fn draw_text_blob(&mut self, id: u32, x: f32, y: f32, p: PaintAttrs) {
        // Multi-run path: each run has its own (bx, by) offset + content hash.
        if let Some(runs) = self.renderer.multi_blob_cache.get(&id).cloned() {
            let paint = make_paint_full(&p, &self.renderer);
            for (blob, bx, by, h) in &runs {
                self.renderer.blit_text_blob_cached(blob, *h, x + bx, y + by, &paint);
            }
            return;
        }
        // Single-run path.
        let Some((blob, content_hash)) = self.renderer.text_blobs.get(&id).cloned() else { return };
        let paint = make_paint_full(&p, &self.renderer);
        self.renderer.blit_text_blob_cached(&blob, content_hash, x, y, &paint);
    }

    fn drop_text_blob(&mut self, id: u32) {
        self.renderer.text_blobs.remove(&id);
        self.renderer.multi_blob_cache.remove(&id);
    }

    fn begin_text_blob(&mut self) {
        self.renderer.text_blob_runs.clear();
    }

    fn add_text_run(&mut self, text: Vec<u8>, font_family: Vec<u8>,
                    size: f32, weight: u32, italic: bool, x: f32, y: f32) {
        self.renderer.text_blob_runs.push(TextBlobRun {
            text:   String::from_utf8_lossy(&text).into_owned(),
            family: String::from_utf8_lossy(&font_family).into_owned(),
            size, weight, italic, x, y,
        });
    }

    fn end_text_blob(&mut self) -> u32 {
        let runs = std::mem::take(&mut self.renderer.text_blob_runs);
        let id = self.renderer.next_blob_id;
        self.renderer.next_blob_id = id.wrapping_add(1).max(1);
        let blobs: Vec<(skia_safe::TextBlob, f32, f32, u64)> = runs.iter().filter_map(|r| {
            let tf = self.renderer.get_typeface(&r.family, r.weight >= 600, r.italic);
            let mut font = Font::new(tf, r.size);
            font.set_edging(skia_safe::font::Edging::AntiAlias);
            font.set_subpixel(false);
            let h = text_content_hash(&r.text, &r.family, r.size, r.weight, r.italic);
            skia_safe::TextBlob::from_str(&r.text, &font).map(|b| (b, r.x, r.y, h))
        }).collect();
        self.renderer.multi_blob_cache.insert(id, blobs);
        id
    }

    // ── images ────────────────────────────────────────────────────────────

    fn create_image(&mut self, width: u32, height: u32, pixels: Vec<u8>) -> u32 {
        let info = skia_safe::ImageInfo::new(
            (width as i32, height as i32),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );
        let data = skia_safe::Data::new_copy(&pixels);
        let id = self.renderer.next_blob_id;
        self.renderer.next_blob_id = id.wrapping_add(1).max(1);
        if let Some(img) = skia_safe::images::raster_from_data(
            &info, data, (width * 4) as usize,
        ) {
            self.renderer.images.insert(id, img);
        }
        id
    }

    fn draw_image(&mut self, id: u32, x: f32, y: f32, alpha: u8) {
        let Some(img) = self.renderer.images.get(&id).cloned() else { return };
        let mut p = Paint::default();
        p.set_alpha(alpha);
        self.renderer.canvas().draw_image(&img, (x, y), Some(&p));
    }

    fn draw_image_rect(&mut self, image_id: u32,
                       src_x: f32, src_y: f32, src_w: f32, src_h: f32,
                       dst_x: f32, dst_y: f32, dst_w: f32, dst_h: f32,
                       p: PaintAttrs) {
        let Some(img) = self.renderer.images.get(&image_id).cloned() else { return };
        let paint = make_paint_full(&p, &self.renderer);
        let src = Rect::from_xywh(src_x, src_y, src_w, src_h);
        let dst = Rect::from_xywh(dst_x, dst_y, dst_w, dst_h);
        self.renderer.canvas().draw_image_rect(
            &img,
            Some((&src, skia_safe::canvas::SrcRectConstraint::Fast)),
            dst, &paint,
        );
    }

    fn drop_image(&mut self, id: u32) {
        self.renderer.images.remove(&id);
    }

    // ── shaders ───────────────────────────────────────────────────────────

    fn create_linear_gradient(&mut self,
        x0: f32, y0: f32, x1: f32, y1: f32,
        colors: Vec<u32>, stops: Vec<f32>, tile_mode: u8,
    ) -> u32 {
        let p0 = skia_safe::Point::new(x0, y0);
        let p1 = skia_safe::Point::new(x1, y1);
        let cols: Vec<skia_safe::Color> = colors.iter().map(|&c| Color::new(c)).collect();
        let stops_opt: Option<&[f32]> = if stops.is_empty() { None } else { Some(&stops) };
        let mode = tile_mode_from_u8(tile_mode);
        let shader = skia_safe::gradient_shader::linear(
            (p0, p1), cols.as_slice(), stops_opt, mode, None, None,
        );
        if let Some(s) = shader {
            let id = self.renderer.next_shader_id;
            self.renderer.next_shader_id = id.wrapping_add(1).max(1);
            self.renderer.shader_cache.insert(id, s);
            id
        } else { 0 }
    }

    fn create_radial_gradient(&mut self,
        cx: f32, cy: f32, radius: f32,
        colors: Vec<u32>, stops: Vec<f32>, tile_mode: u8,
    ) -> u32 {
        let center = skia_safe::Point::new(cx, cy);
        let cols: Vec<skia_safe::Color> = colors.iter().map(|&c| Color::new(c)).collect();
        let stops_opt: Option<&[f32]> = if stops.is_empty() { None } else { Some(&stops) };
        let mode = tile_mode_from_u8(tile_mode);
        let shader = skia_safe::gradient_shader::radial(
            center, radius, cols.as_slice(), stops_opt, mode, None, None,
        );
        if let Some(s) = shader {
            let id = self.renderer.next_shader_id;
            self.renderer.next_shader_id = id.wrapping_add(1).max(1);
            self.renderer.shader_cache.insert(id, s);
            id
        } else { 0 }
    }

    fn drop_shader(&mut self, id: u32) {
        self.renderer.shader_cache.remove(&id);
    }

    // ── picture recording (Tier A skia shim) ──────────────────────────────

    fn create_picture_recorder(&mut self) -> u32 {
        let id = self.renderer.next_recorder_id;
        self.renderer.next_recorder_id = id.wrapping_add(1).max(1);
        self.renderer.recorders.insert(id, skia_safe::PictureRecorder::new());
        id
    }

    fn begin_picture_recording(&mut self, recorder_id: u32,
                                left: f32, top: f32, right: f32, bottom: f32,
                                with_rtree: bool) {
        // with_rtree=true asks skia to build a bounding-box hierarchy (RTree)
        // for the picture so partial-replay (drawing the picture clipped to
        // a sub-rect) can skip culled commands. Compose's LegacyRenderNodeLayer
        // / RecordDrawRectRenderDecorator both want this when measuring draw
        // bounds. skia-safe 0.9x exposes the choice as a bool.
        let bounds = Rect::from_ltrb(left, top, right, bottom);
        if let Some(rec) = self.renderer.recorders.get_mut(&recorder_id) {
            let _ = rec.begin_recording(bounds, with_rtree);
            self.renderer.recording_stack.push(recorder_id);
        }
    }

    fn finish_recording_as_picture(&mut self, recorder_id: u32) -> u32 {
        // Pop the stack first so subsequent canvas() lookups don't try to
        // borrow the recorder we're about to consume.
        if let Some(pos) = self.renderer.recording_stack.iter().rposition(|&r| r == recorder_id) {
            self.renderer.recording_stack.remove(pos);
        }
        let pic = self.renderer.recorders.get_mut(&recorder_id)
            .and_then(|r| r.finish_recording_as_picture(None));
        match pic {
            Some(p) => {
                let id = self.renderer.next_picture_id;
                self.renderer.next_picture_id = id.wrapping_add(1).max(1);
                self.renderer.pictures.insert(id, p);
                id
            }
            None => 0,
        }
    }

    fn close_picture_recorder(&mut self, recorder_id: u32) {
        // If still on the recording stack, pop it.
        if let Some(pos) = self.renderer.recording_stack.iter().rposition(|&r| r == recorder_id) {
            self.renderer.recording_stack.remove(pos);
        }
        self.renderer.recorders.remove(&recorder_id);
    }

    fn draw_picture(&mut self, picture_id: u32) {
        let pic = self.renderer.pictures.get(&picture_id).cloned();
        if let Some(pic) = pic {
            self.renderer.canvas().draw_picture(&pic, None, None);
        }
    }

    fn drop_picture(&mut self, picture_id: u32) {
        self.renderer.pictures.remove(&picture_id);
    }

    // ── WasiDrawable (deferred-replay) ─────────────────────────────────────

    fn create_drawable(&mut self) -> u32 {
        let id = self.renderer.next_drawable_id;
        self.renderer.next_drawable_id = id.wrapping_add(1).max(1);
        self.renderer.drawables.insert(id, WasiDrawable::new());
        id
    }

    fn set_drawable_from_recorder(&mut self, drawable_id: u32, recorder_id: u32) {
        // Pop the recorder off the recording stack if present (matches
        // finish_recording_as_picture). This pairs the begin/end bracket
        // so subsequent draw ops route to the screen surface again.
        if let Some(pos) = self.renderer.recording_stack.iter()
            .rposition(|&r| r == recorder_id)
        {
            self.renderer.recording_stack.remove(pos);
        }
        // finish_recording_as_drawable detaches the SkRecord + drawable
        // list (no picture snapshot is taken), so embedded child
        // drawables stay live across parent re-records.
        let inner: Option<skia_safe::Drawable> = self.renderer.recorders
            .get_mut(&recorder_id)
            .and_then(|r| r.finish_recording_as_drawable());
        let inner_ptr: *mut std::os::raw::c_void = match inner.as_ref() {
            Some(d) => handle_to_native_ptr(d as *const skia_safe::Drawable),
            None    => std::ptr::null_mut(),
        };
        if let Some(outer) = self.renderer.drawables.get(&drawable_id) {
            unsafe {
                wasi_drawable_ffi::wasi_drawable_set_inner(outer.as_ptr(), inner_ptr);
            }
        }
        // `inner` drops here, releasing its handle ref; the outer's
        // setInner already bumped the underlying SkDrawable's refcount
        // via sk_ref_sp, so the drawable stays alive.
        drop(inner);
    }

    fn set_drawable_bounds(&mut self, drawable_id: u32,
                           l: f32, t: f32, r: f32, b: f32) {
        if let Some(d) = self.renderer.drawables.get_mut(&drawable_id) {
            d.set_bounds(l, t, r, b);
        }
    }

    fn draw_drawable(&mut self, drawable_id: u32) {
        // We dispatch via our own C FFI (wasi_canvas_draw_drawable) rather
        // than skia_safe::Canvas::draw_drawable because Drawable::from_ptr
        // is pub(crate) in skia-safe. skia_safe::Canvas is
        // `pub struct Canvas(UnsafeCell<SkCanvas>)` — a transparent
        // single-field wrapper around SkCanvas, so its first byte coincides
        // with the SkCanvas* and casting through *mut c_void is sound.
        let raw_d = match self.renderer.drawables.get(&drawable_id) {
            Some(d) => d.as_ptr(),
            None    => return,
        };
        if raw_d.is_null() { return; }
        let canvas: &skia_safe::Canvas = self.renderer.canvas();
        let canvas_ptr = canvas as *const skia_safe::Canvas as *mut std::os::raw::c_void;
        unsafe { wasi_drawable_ffi::wasi_canvas_draw_drawable(canvas_ptr, raw_d); }
    }

    fn drop_drawable(&mut self, drawable_id: u32) {
        self.renderer.drawables.remove(&drawable_id);
    }

    fn set_drawable_transform(
        &mut self, drawable_id: u32,
        layer_x: f32, layer_y: f32,
        translation_x: f32, translation_y: f32,
        scale_x: f32, scale_y: f32,
        rotation_z: f32,
        pivot_x: f32, pivot_y: f32,
        alpha: f32,
    ) {
        let raw_d = match self.renderer.drawables.get(&drawable_id) {
            Some(d) => d.as_ptr(),
            None    => return,
        };
        unsafe { wasi_drawable_ffi::wasi_drawable_set_transform(
            raw_d, layer_x, layer_y, translation_x, translation_y,
            scale_x, scale_y, rotation_z, pivot_x, pivot_y, alpha,
        ); }
    }

    fn set_drawable_clip_rect(
        &mut self, drawable_id: u32,
        l: f32, t: f32, r: f32, b: f32, antialias: bool,
    ) {
        let raw_d = match self.renderer.drawables.get(&drawable_id) {
            Some(d) => d.as_ptr(),
            None    => return,
        };
        unsafe { wasi_drawable_ffi::wasi_drawable_set_clip_rect(
            raw_d, l, t, r, b, antialias,
        ); }
    }

    fn set_drawable_clip_rrect(
        &mut self, drawable_id: u32,
        l: f32, t: f32, r: f32, b: f32, radii: Vec<f32>, antialias: bool,
    ) {
        let raw_d = match self.renderer.drawables.get(&drawable_id) {
            Some(d) => d.as_ptr(),
            None    => return,
        };
        // C++ side expects exactly 8 floats (4 corners × (rx, ry)). Skiko's
        // RRect.makeComplexLTRB also stores them in upper-left → upper-right
        // → lower-right → lower-left order matching SkRRect::setRectRadii.
        if radii.len() < 8 { return; }
        unsafe { wasi_drawable_ffi::wasi_drawable_set_clip_rrect(
            raw_d, l, t, r, b, radii.as_ptr(), antialias,
        ); }
    }

    fn clear_drawable_clip(&mut self, drawable_id: u32) {
        let raw_d = match self.renderer.drawables.get(&drawable_id) {
            Some(d) => d.as_ptr(),
            None    => return,
        };
        unsafe { wasi_drawable_ffi::wasi_drawable_clear_clip(raw_d); }
    }

    fn set_drawable_shadow_elevation(&mut self, drawable_id: u32, elevation: f32) {
        let raw_d = match self.renderer.drawables.get(&drawable_id) {
            Some(d) => d.as_ptr(),
            None    => return,
        };
        unsafe { wasi_drawable_ffi::wasi_drawable_set_shadow_elevation(raw_d, elevation); }
    }

    // ── debug log ─────────────────────────────────────────────────────────

    fn log_message(&mut self, msg: String) {
        log::info!("[wasm] {}", msg);
    }
}

// ─── Paint helpers ───────────────────────────────────────────────────────────

fn make_paint(attrs: &PaintAttrs) -> Paint {
    let mut p = Paint::default();
    p.set_argb(
        ((attrs.color >> 24) & 0xFF) as u8,
        ((attrs.color >> 16) & 0xFF) as u8,
        ((attrs.color >>  8) & 0xFF) as u8,
        ( attrs.color        & 0xFF) as u8,
    );
    p.set_style(match attrs.style {
        WitPaintStyle::Fill          => PaintStyle::Fill,
        WitPaintStyle::Stroke        => PaintStyle::Stroke,
        WitPaintStyle::FillAndStroke => PaintStyle::StrokeAndFill,
    });
    p.set_stroke_width(attrs.stroke_width);
    p.set_stroke_miter(attrs.stroke_miter);
    p.set_stroke_cap(match attrs.stroke_cap {
        StrokeCap::Butt   => skia_safe::PaintCap::Butt,
        StrokeCap::Round  => skia_safe::PaintCap::Round,
        StrokeCap::Square => skia_safe::PaintCap::Square,
    });
    p.set_stroke_join(match attrs.stroke_join {
        StrokeJoin::Miter => skia_safe::PaintJoin::Miter,
        StrokeJoin::Round => skia_safe::PaintJoin::Round,
        StrokeJoin::Bevel => skia_safe::PaintJoin::Bevel,
    });
    p.set_anti_alias(attrs.anti_alias);
    p.set_alpha(attrs.alpha);
    p.set_blend_mode(match attrs.blend_mode {
        BlendMode::SrcOver    => skia_safe::BlendMode::SrcOver,
        BlendMode::Src        => skia_safe::BlendMode::Src,
        BlendMode::DstIn      => skia_safe::BlendMode::DstIn,
        BlendMode::DstOut     => skia_safe::BlendMode::DstOut,
        BlendMode::SrcAtop    => skia_safe::BlendMode::SrcATop,
        BlendMode::DstAtop    => skia_safe::BlendMode::DstATop,
        BlendMode::Xor        => skia_safe::BlendMode::Xor,
        BlendMode::Multiply   => skia_safe::BlendMode::Multiply,
        BlendMode::Screen     => skia_safe::BlendMode::Screen,
        BlendMode::Overlay    => skia_safe::BlendMode::Overlay,
        BlendMode::Darken     => skia_safe::BlendMode::Darken,
        BlendMode::Lighten    => skia_safe::BlendMode::Lighten,
        BlendMode::ColorDodge => skia_safe::BlendMode::ColorDodge,
        BlendMode::ColorBurn  => skia_safe::BlendMode::ColorBurn,
        BlendMode::HardLight  => skia_safe::BlendMode::HardLight,
        BlendMode::SoftLight  => skia_safe::BlendMode::SoftLight,
        BlendMode::Difference => skia_safe::BlendMode::Difference,
        BlendMode::Exclusion  => skia_safe::BlendMode::Exclusion,
        BlendMode::Clear      => skia_safe::BlendMode::Clear,
    });
    p
}

fn make_paint_full(attrs: &PaintAttrs, renderer: &SkiaRenderer) -> Paint {
    let mut p = make_paint(attrs);
    // Shader
    if attrs.shader_id != 0 {
        if let Some(s) = renderer.shader_cache.get(&attrs.shader_id) {
            p.set_shader(Some(s.clone()));
        }
    }
    // Color filter
    match attrs.color_filter_kind {
        ColorFilterKind::Blend => {
            let c = attrs.color_filter_color;
            let color = skia_safe::Color::from_argb(
                (c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8, c as u8);
            if let Some(cf) = skia_safe::color_filters::blend(color, skia_safe::BlendMode::Modulate) {
                p.set_color_filter(cf);
            }
        }
        ColorFilterKind::Invert => {
            let matrix = [
                -1f32,  0f32,  0f32, 0f32, 1f32,
                 0f32, -1f32,  0f32, 0f32, 1f32,
                 0f32,  0f32, -1f32, 0f32, 1f32,
                 0f32,  0f32,  0f32, 1f32, 0f32,
            ];
            let cf = skia_safe::color_filters::matrix_row_major(&matrix, None);
            p.set_color_filter(cf);
        }
        ColorFilterKind::None => {}
    }
    p
}

fn tile_mode_from_u8(m: u8) -> skia_safe::TileMode {
    match m {
        1 => skia_safe::TileMode::Repeat,
        2 => skia_safe::TileMode::Mirror,
        3 => skia_safe::TileMode::Decal,
        _ => skia_safe::TileMode::Clamp,
    }
}
