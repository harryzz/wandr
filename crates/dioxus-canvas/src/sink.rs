//! The host boundary. `dioxus-canvas` never touches WIT directly — it emits
//! paint commands through this trait. A guest wandrpkg implements it by forwarding
//! to its own `wit_bindgen`-generated `my:skiko-gfx/canvas` imports (the same
//! verbs `wandr.launcher` uses, plus `measure-text`).
//!
//! Keeping the renderer WIT-agnostic means it builds + unit-tests on the host
//! against a mock sink, and a guest can swap in a trimmed WIT without the
//! renderer caring about field/enum ordering (that lives in the guest's WIT).

/// A solid-fill paint description. The renderer only ever needs flat fills and
/// rounded rects for the box model + text, so this is intentionally tiny — the
/// guest expands it into a full `paint-attrs` record when forwarding.
#[derive(Clone, Copy, Debug)]
pub struct Fill {
    /// ARGB, e.g. `0xFF1A1A2E`.
    pub color: u32,
}

/// What the renderer needs from the host canvas. All coordinates are absolute
/// device pixels in the surface's logical space (already inset by the host for
/// fullscreen apps — task 56).
pub trait CanvasSink {
    /// Logical surface dimensions. Called once per layout pass.
    fn surface_size(&mut self) -> (f32, f32);

    /// Frame boundaries — map 1:1 to the canvas WIT `begin-frame`/`end-frame`.
    fn begin_frame(&mut self);
    fn end_frame(&mut self);
    /// Clear the whole surface to an ARGB colour.
    fn clear(&mut self, argb: u32);

    /// Canvas save/restore + rectangular clip — used to make scroll regions
    /// clip their (scrolled) content so siblings (e.g. a sticky header) aren't
    /// overdrawn. Map 1:1 to the canvas WIT `save`/`restore`/`clip-rect`.
    fn save(&mut self);
    fn restore(&mut self);
    fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32);

    /// Filled rectangle.
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Fill);
    /// Filled rounded rectangle (`rx`/`ry` corner radii).
    fn fill_rrect(&mut self, x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32, fill: Fill);

    /// Create a host-owned text blob, returning its id. Mirrors
    /// `canvas::create-text-blob`. `weight` is a CSS-ish 100..900.
    fn create_text_blob(&mut self, text: &str, family: &str, size: f32, weight: u32, italic: bool) -> u32;
    /// Draw a previously created blob at a baseline origin.
    fn draw_text_blob(&mut self, id: u32, x: f32, y: f32, fill: Fill);
    /// Release a blob (called when the previous layout's blobs are discarded).
    fn drop_text_blob(&mut self, id: u32);

    /// Measure a text run without drawing it. Returns `(width, height)` in
    /// device pixels. Backed by the new host `measure-text` WIT verb (Skia owns
    /// fonts, so the guest must never measure in-process — see
    /// `feedback_android_fonts`).
    fn measure_text(&mut self, text: &str, family: &str, size: f32, weight: u32, italic: bool) -> (f32, f32);

    /// Decode encoded image bytes (PNG/JPEG/WebP/…) into a host image, returning
    /// its id (`0` on decode failure). Mirrors `canvas::create-image-from-encoded`.
    /// The renderer calls this for an `<img src="data:…;base64,…">` and caches the
    /// result by content (see `DomRenderer`).
    fn create_image(&mut self, bytes: &[u8]) -> u32;

    /// Draw host image `id` scaled to fill the dst rect (full source → dst box).
    /// Mirrors `canvas::draw-image-rect`.
    fn draw_image_rect(&mut self, id: u32, x: f32, y: f32, w: f32, h: f32);
}
