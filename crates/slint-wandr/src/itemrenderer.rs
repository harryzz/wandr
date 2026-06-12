//! The `ItemRenderer` that draws Slint items through the **wasi:canvas
//! draft** (proposals/wasi-canvas) — the proving consumer. The embedder
//! hands `render()` a `canvas` resource for the frame plus the long-lived
//! `graphics` factory; everything here are methods on those handles.
//!
//! Fidelity upgrades over the original my:skiko-gfx port (task 100), all
//! enabled by the draft's types: PER-CORNER border radii (no more
//! max-corner approximation), real fill rules on paths, box shadows as a
//! plain `paint.blur` (no fused verb), and shaders that drop themselves
//! (owned resources) instead of create/drop-id bookkeeping.
//!
//! Coordinates: Slint hands the renderer LOGICAL units and a scale factor;
//! the canvas draws in physical surface units, so every geometry value is
//! multiplied by `scale` at the call site. Clip tracking stays guest-side
//! (the femtovg pattern): translate/scale fold into the tracked rect,
//! rotation degrades it to "unbounded" so culling stays conservative.

use std::cell::RefCell;
use std::collections::HashMap;

use i_slint_core::Brush;
use i_slint_core::graphics::euclid;
use i_slint_core::graphics::{ImageCacheKey, SharedImageBuffer};
use i_slint_core::items::ItemRc;
use i_slint_core::lengths::{
    LogicalBorderRadius, LogicalLength, LogicalPoint, LogicalRect, LogicalSize, LogicalVector,
    ScaleFactor,
};
use i_slint_core::textlayout::sharedparley::{self, GlyphRenderer, fontique};
use i_slint_core::window::WindowInner;

use crate::{wdraw, wglyphs, wtypes};

/// An "infinite" clip: after a rotation the tracked rect is no longer
/// axis-aligned in local coords, so culling falls back to "draw everything"
/// (the actual canvas clip on the host stays exact).
fn unbounded_clip() -> LogicalRect {
    LogicalRect::new(
        LogicalPoint::new(f32::MIN / 2.0, f32::MIN / 2.0),
        LogicalSize::new(f32::MAX, f32::MAX),
    )
}

thread_local! {
    /// Host image resources by Slint image cache key (+ render size — it
    /// matters for scalable SVG sources). Owned handles live here, so the
    /// host-side images persist across frames.
    static IMAGE_CACHE: RefCell<HashMap<(ImageCacheKey, u32, u32), wtypes::Image>> =
        RefCell::new(HashMap::new());

    /// Host typeface resources by font blob identity (ptr, len, index).
    /// The cached `FontData` clone PINS the blob (Slint's HashedBlob trick)
    /// so the ptr-keyed entry stays valid; glyph ids stay meaningful
    /// because the host typeface is built from these exact bytes.
    static TYPEFACE_CACHE: RefCell<
        HashMap<(usize, usize, u32), (wglyphs::Typeface, sharedparley::parley::FontData)>,
    > = RefCell::new(HashMap::new());
}

/// Host typeface resource for a parley font blob (created once, cached).
/// Returns the typeface by running `f` against the cache entry (handles
/// are owned by the cache; callers borrow within the closure).
fn with_typeface<R>(
    font: &sharedparley::parley::FontData,
    f: impl FnOnce(&wglyphs::Typeface) -> R,
) -> Option<R> {
    let bytes: &[u8] = font.data.as_ref();
    let key = (bytes.as_ptr() as usize, bytes.len(), font.index);
    TYPEFACE_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        if !cache.contains_key(&key) {
            match wglyphs::Typeface::from_bytes(bytes, font.index) {
                Ok(tf) => {
                    cache.insert(key, (tf, font.clone()));
                }
                Err(()) => {
                    eprintln!("slint-wandr: typeface.from-bytes failed (unparseable blob)");
                    return None;
                }
            }
        }
        cache.get(&key).map(|(tf, _)| f(tf))
    })
}

#[derive(Clone)]
struct RenderState {
    alpha: f32,
    clip: LogicalRect,
    /// Canvas saves issued beyond the save_state() one (opacity layers),
    /// popped by the matching restore_state().
    extra_canvas_saves: u32,
}

/// Owned paint spec (no resource borrows) — the `Clone`-able brush type
/// the GlyphRenderer trait needs; lowered to a `wtypes::Paint` at draw.
#[derive(Clone)]
pub struct TextBrush {
    color: u32,
    stroke_width: f32,
    stroke: bool,
}

pub struct WandrItemRenderer<'a> {
    window: &'a i_slint_core::api::Window,
    scale: f32,
    text_layout_cache: &'a sharedparley::TextLayoutCache,
    canvas: &'a wdraw::Canvas,
    graphics: &'a wdraw::Graphics,
    state: RenderState,
    saved: Vec<RenderState>,
}

fn point(x: f32, y: f32) -> wtypes::Point {
    wtypes::Point { x, y }
}

fn wrect(x: f32, y: f32, w: f32, h: f32) -> wtypes::Rect {
    wtypes::Rect { x, y, width: w, height: h }
}

/// Slint per-corner radius → the draft's per-corner rounded-rect (the
/// task-100 max-corner approximation retires here).
fn wrrect(x: f32, y: f32, w: f32, h: f32, r: &LogicalBorderRadius, scale: f32) -> wtypes::RoundedRect {
    let c = |v: f32| {
        let v = (v * scale).max(0.0);
        point(v, v)
    };
    wtypes::RoundedRect {
        rect: wrect(x, y, w, h),
        top_left: c(r.top_left),
        top_right: c(r.top_right),
        bottom_right: c(r.bottom_right),
        bottom_left: c(r.bottom_left),
    }
}

fn color_argb(c: i_slint_core::Color) -> u32 {
    ((c.alpha() as u32) << 24)
        | ((c.red() as u32) << 16)
        | ((c.green() as u32) << 8)
        | (c.blue() as u32)
}

fn base_paint(
    color: u32,
    style: wtypes::PaintStyle,
    stroke_width: f32,
    alpha: u8,
) -> wtypes::Paint<'static> {
    wtypes::Paint {
        style,
        color,
        alpha,
        blend: wtypes::BlendMode::SrcOver,
        anti_alias: true,
        shader: None,
        stroke_width,
        stroke_cap: wtypes::StrokeCap::Butt,
        stroke_join: wtypes::StrokeJoin::Miter,
        stroke_miter: 4.0,
        blur: None,
        filter: None,
    }
}

/// Gradient brushes → owned shader resources (Clamp tiling; geometry per
/// the upstream renderers: linear via `line_for_angle`, radial scaled
/// center/radius, conic = full sweep with 0° at 12 o'clock — the draft's
/// sweep angles start at the +x axis, hence -90).
fn make_brush_shader(
    graphics: &wdraw::Graphics,
    brush: &Brush,
    w: f32,
    h: f32,
    scale: f32,
) -> Option<wtypes::Shader> {
    let stops = |it: &mut dyn Iterator<Item = (f32, u32)>| -> Vec<(f32, u32)> { it.collect() };
    match brush {
        Brush::LinearGradient(g) => {
            let (start, end) = i_slint_core::graphics::line_for_angle(g.angle(), [w, h].into());
            let s = stops(&mut g.stops().map(|s| (s.position, color_argb(s.color))));
            Some(graphics.linear_gradient(
                point(start.x, start.y),
                point(end.x, end.y),
                &s,
                wtypes::TileMode::Clamp,
                None,
            ))
        }
        Brush::RadialGradient(g) => {
            let (cx, cy) = g.center_or_default_scaled(w, h, scale);
            let radius = g.radius_or_default_scaled(w, h, scale);
            let s = stops(&mut g.stops().map(|s| (s.position, color_argb(s.color))));
            Some(graphics.radial_gradient(point(cx, cy), radius, &s, wtypes::TileMode::Clamp, None))
        }
        Brush::ConicGradient(g) => {
            let (cx, cy) = g.center_or_default_scaled(w, h, scale);
            let s = stops(&mut g.stops().map(|s| (s.position, color_argb(s.color))));
            Some(graphics.sweep_gradient(point(cx, cy), -90.0, 270.0, &s, wtypes::TileMode::Clamp, None))
        }
        _ => None,
    }
}

fn linear_sampling() -> wtypes::Sampling {
    wtypes::Sampling { filter: wtypes::FilterMode::Linear, mipmap: wtypes::MipmapMode::None }
}

/// Any Slint pixel buffer → tightly-packed RGBA8 straight-alpha (what
/// `graphics.image-from-rgba8` expects).
fn buffer_to_rgba8_unpremul(buffer: &SharedImageBuffer) -> (u32, u32, Vec<u8>) {
    match buffer {
        SharedImageBuffer::RGB8(b) => {
            let mut out = Vec::with_capacity(b.as_bytes().len() / 3 * 4);
            for px in b.as_bytes().chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            (b.width(), b.height(), out)
        }
        SharedImageBuffer::RGBA8(b) => (b.width(), b.height(), b.as_bytes().to_vec()),
        SharedImageBuffer::RGBA8Premultiplied(b) => {
            let mut out = Vec::with_capacity(b.as_bytes().len());
            for px in b.as_bytes().chunks_exact(4) {
                let a = px[3];
                if a == 0 || a == 255 {
                    out.extend_from_slice(px);
                } else {
                    let un = |v: u8| ((v as u32 * 255) / a as u32).min(255) as u8;
                    out.extend_from_slice(&[un(px[0]), un(px[1]), un(px[2]), a]);
                }
            }
            (b.width(), b.height(), out)
        }
    }
}

/// lyon path events → SVG path string in PHYSICAL units (the draft's
/// SVG-grammar wire format).
fn path_events_to_svg(
    events: impl Iterator<Item = lyon_path::Event<lyon_path::math::Point, lyon_path::math::Point>>,
    scale: f32,
) -> String {
    use std::fmt::Write as _;
    let mut svg = String::new();
    for ev in events {
        match ev {
            lyon_path::Event::Begin { at } => {
                let _ = write!(svg, "M {} {} ", at.x * scale, at.y * scale);
            }
            lyon_path::Event::Line { to, .. } => {
                let _ = write!(svg, "L {} {} ", to.x * scale, to.y * scale);
            }
            lyon_path::Event::Quadratic { ctrl, to, .. } => {
                let _ = write!(
                    svg,
                    "Q {} {} {} {} ",
                    ctrl.x * scale, ctrl.y * scale, to.x * scale, to.y * scale
                );
            }
            lyon_path::Event::Cubic { ctrl1, ctrl2, to, .. } => {
                let _ = write!(
                    svg,
                    "C {} {} {} {} {} {} ",
                    ctrl1.x * scale, ctrl1.y * scale,
                    ctrl2.x * scale, ctrl2.y * scale,
                    to.x * scale, to.y * scale
                );
            }
            lyon_path::Event::End { close, .. } => {
                if close {
                    svg.push_str("Z ");
                }
            }
        }
    }
    svg
}

impl<'a> WandrItemRenderer<'a> {
    pub fn new(
        window: &'a i_slint_core::api::Window,
        scale: f32,
        text_layout_cache: &'a sharedparley::TextLayoutCache,
        canvas: &'a wdraw::Canvas,
        graphics: &'a wdraw::Graphics,
    ) -> Self {
        let size = window.size();
        Self {
            window,
            scale,
            text_layout_cache,
            canvas,
            graphics,
            state: RenderState {
                alpha: 1.0,
                clip: LogicalRect::new(
                    LogicalPoint::default(),
                    LogicalSize::new(size.width as f32 / scale, size.height as f32 / scale),
                ),
                extra_canvas_saves: 0,
            },
            saved: Vec::new(),
        }
    }

    /// Paint the window background under the item tree.
    pub fn fill_window_background(&mut self, brush: Brush) {
        let size = self.window.size();
        self.with_brush_paint(
            &brush,
            size.width as f32,
            size.height as f32,
            wtypes::PaintStyle::Fill,
            0.0,
            |c, p| c.draw_paint(&p),
        );
    }

    /// Build a paint for `brush` (gradients become owned shader resources,
    /// dropped automatically after the draw) and hand it + the canvas to
    /// `f`. No-op for transparent brushes. `w`/`h` are PHYSICAL units.
    fn with_brush_paint(
        &self,
        brush: &Brush,
        w: f32,
        h: f32,
        style: wtypes::PaintStyle,
        stroke_width: f32,
        f: impl FnOnce(&wdraw::Canvas, wtypes::Paint<'_>),
    ) {
        if brush.is_transparent() {
            return;
        }
        let shader = make_brush_shader(self.graphics, brush, w, h, self.scale);
        let mut p = base_paint(
            color_argb(brush.color()),
            style,
            stroke_width,
            (self.state.alpha * 255.0) as u8,
        );
        p.shader = shader.as_ref();
        f(self.canvas, p);
        // `shader` drops here → host resource freed.
    }

    /// Resolve a Slint image to a cached host image resource and run `f`
    /// on it (handles are owned by the cache).
    fn with_image<R>(
        &self,
        image: &i_slint_core::graphics::Image,
        target: (u32, u32),
        f: impl FnOnce(&wtypes::Image) -> R,
    ) -> Option<R> {
        let inner: &i_slint_core::ImageInner = image.into();
        let key = ImageCacheKey::new(inner).map(|k| (k, target.0, target.1))?;
        IMAGE_CACHE.with(|c| {
            let mut cache = c.borrow_mut();
            if !cache.contains_key(&key) {
                let buffer = inner.render_to_buffer(Some(euclid::Size2D::new(
                    target.0.max(1),
                    target.1.max(1),
                )))?;
                let (w, h, rgba) = buffer_to_rgba8_unpremul(&buffer);
                if w == 0 || h == 0 {
                    return None;
                }
                match self.graphics.image_from_rgba8(w, h, &rgba) {
                    Ok(img) => {
                        cache.insert(key.clone(), img);
                    }
                    Err(()) => return None,
                }
            }
            cache.get(&key).map(f)
        })
    }
}

impl i_slint_core::item_rendering::ItemRenderer for WandrItemRenderer<'_> {
    fn draw_rectangle(
        &mut self,
        rect: core::pin::Pin<&dyn i_slint_core::item_rendering::RenderRectangle>,
        _self_rc: &ItemRc,
        size: LogicalSize,
        _cache: &i_slint_core::item_rendering::CachedRenderingData,
    ) {
        let s = self.scale;
        let (w, h) = (size.width * s, size.height * s);
        self.with_brush_paint(&rect.background(), w, h, wtypes::PaintStyle::Fill, 0.0, |c, p| {
            c.draw_rect(wrect(0.0, 0.0, w, h), &p);
        });
    }

    fn draw_border_rectangle(
        &mut self,
        rect: core::pin::Pin<&dyn i_slint_core::item_rendering::RenderBorderRectangle>,
        _self_rc: &ItemRc,
        size: LogicalSize,
        _cache: &i_slint_core::item_rendering::CachedRenderingData,
    ) {
        let s = self.scale;
        let radius = rect.border_radius();
        let (w, h) = (size.width * s, size.height * s);

        self.with_brush_paint(&rect.background(), w, h, wtypes::PaintStyle::Fill, 0.0, |c, p| {
            c.draw_rounded_rect(wrrect(0.0, 0.0, w, h, &radius, s), &p);
        });

        let border_width = rect.border_width().get() * s;
        let border_color = rect.border_color();
        if border_width > 0.0 && !border_color.is_transparent() {
            // Stroke centered on the inset midline so the border stays
            // inside the item bounds (Slint's border model).
            let inset = border_width / 2.0;
            let inner = LogicalBorderRadius::new(
                (radius.top_left - inset / s).max(0.0),
                (radius.top_right - inset / s).max(0.0),
                (radius.bottom_right - inset / s).max(0.0),
                (radius.bottom_left - inset / s).max(0.0),
            );
            self.with_brush_paint(
                &border_color,
                w,
                h,
                wtypes::PaintStyle::Stroke,
                border_width,
                |c, p| {
                    c.draw_rounded_rect(
                        wrrect(inset, inset, w - border_width, h - border_width, &inner, s),
                        &p,
                    );
                },
            );
        }
    }

    fn draw_window_background(
        &mut self,
        rect: core::pin::Pin<&dyn i_slint_core::item_rendering::RenderRectangle>,
        self_rc: &ItemRc,
        size: LogicalSize,
        cache: &i_slint_core::item_rendering::CachedRenderingData,
    ) {
        self.draw_rectangle(rect, self_rc, size, cache);
    }

    fn draw_image(
        &mut self,
        image: core::pin::Pin<&dyn i_slint_core::item_rendering::RenderImage>,
        _self_rc: &ItemRc,
        size: LogicalSize,
        _cache: &i_slint_core::item_rendering::CachedRenderingData,
    ) {
        let s = self.scale;
        let source = image.source();
        let source_size = source.size();
        if source_size.is_empty() {
            return;
        }
        let dest = euclid::Size2D::new(size.width * s, size.height * s);
        let fit = i_slint_core::graphics::fit(
            image.image_fit(),
            dest,
            image
                .source_clip()
                .unwrap_or_else(|| euclid::Rect::from_size(source_size.cast())),
            ScaleFactor::new(s),
            image.alignment(),
            image.tiling(), // tiled output degrades to a stretched draw
        );
        let alpha = (self.state.alpha * 255.0) as u8;
        let canvas = self.canvas;
        self.with_image(
            &source,
            (dest.width.ceil() as u32, dest.height.ceil() as u32),
            |img| {
                let (iw, ih) = (img.width(), img.height());
                if iw == 0 || ih == 0 {
                    return;
                }
                // The host image may differ from source.size() (SVG rendered
                // at target size) — map the source clip proportionally.
                let sx = iw as f32 / source_size.width as f32;
                let sy = ih as f32 / source_size.height as f32;
                let mut p = base_paint(0xFFFF_FFFF, wtypes::PaintStyle::Fill, 0.0, alpha);
                // Colorize: the draft has no color-filter (deliberate);
                // approximate via alpha-only (rare; full support = shader
                // blend if a real consumer needs it).
                let _ = &mut p;
                canvas.save();
                canvas.clip_rect(
                    wrect(fit.offset.x, fit.offset.y, fit.size.width, fit.size.height),
                    true,
                );
                canvas.draw_image_rect(
                    img,
                    wrect(
                        fit.clip_rect.origin.x as f32 * sx,
                        fit.clip_rect.origin.y as f32 * sy,
                        fit.clip_rect.size.width as f32 * sx,
                        fit.clip_rect.size.height as f32 * sy,
                    ),
                    wrect(
                        fit.offset.x,
                        fit.offset.y,
                        fit.clip_rect.size.width as f32 * fit.source_to_target_x,
                        fit.clip_rect.size.height as f32 * fit.source_to_target_y,
                    ),
                    linear_sampling(),
                    &p,
                );
                canvas.restore();
            },
        );
    }

    fn draw_text(
        &mut self,
        text: core::pin::Pin<&dyn i_slint_core::item_rendering::RenderText>,
        self_rc: &ItemRc,
        size: LogicalSize,
        _cache: &i_slint_core::item_rendering::CachedRenderingData,
    ) {
        sharedparley::draw_text(self, text, Some(self_rc), size, Some(self.text_layout_cache));
    }

    fn draw_text_input(
        &mut self,
        text_input: core::pin::Pin<&i_slint_core::items::TextInput>,
        self_rc: &ItemRc,
        size: LogicalSize,
    ) {
        sharedparley::draw_text_input(self, text_input, self_rc, size, None);
    }

    fn draw_path(
        &mut self,
        path: core::pin::Pin<&i_slint_core::items::Path>,
        item_rc: &ItemRc,
        _size: LogicalSize,
    ) {
        let Some((offset, events)) = path.fitted_path_events(item_rc) else { return };
        let svg = path_events_to_svg(events.iter(), self.scale);
        if svg.is_empty() {
            return;
        }
        // Real fill rules now (the draft carries them on draw-path).
        let rule = match path.fill_rule() {
            i_slint_core::items::FillRule::Evenodd => wtypes::FillRule::Evenodd,
            _ => wtypes::FillRule::Nonzero,
        };
        let s = self.scale;
        self.canvas.save();
        self.canvas.translate(offset.x * s, offset.y * s);

        let (gw, gh) = (path.viewbox_width() * s, path.viewbox_height() * s);
        let fill = path.fill();
        if !fill.is_transparent() {
            self.with_brush_paint(&fill, gw, gh, wtypes::PaintStyle::Fill, 0.0, |c, p| {
                c.draw_path(&svg, rule, &p);
            });
        }
        let stroke = path.stroke();
        let stroke_width = path.stroke_width().get() * s;
        if !stroke.is_transparent() && stroke_width > 0.0 {
            self.with_brush_paint(
                &stroke,
                gw,
                gh,
                wtypes::PaintStyle::Stroke,
                stroke_width,
                |c, p| {
                    c.draw_path(&svg, rule, &p);
                },
            );
        }
        self.canvas.restore();
    }

    fn draw_box_shadow(
        &mut self,
        box_shadow: core::pin::Pin<&i_slint_core::items::BoxShadow>,
        _self_rc: &ItemRc,
        size: LogicalSize,
    ) {
        if box_shadow.color().alpha() == 0 {
            return;
        }
        let s = self.scale;
        let spread = box_shadow.spread().get() * s;
        let blur = box_shadow.blur().get() * s;
        let x = box_shadow.offset_x().get() * s - spread;
        let y = box_shadow.offset_y().get() * s - spread;
        let w = size.width * s + 2.0 * spread;
        let h = size.height * s + 2.0 * spread;
        // CSS rule: outer corner radius after spread = max(0, r + spread);
        // Slint blur → Gaussian sigma = blur/2 (upstream skia convention).
        let radius = box_shadow.logical_border_radius()
            + LogicalBorderRadius::new_uniform(spread / s);
        let mut color = color_argb(box_shadow.color());
        if self.state.alpha < 1.0 {
            let a = ((color >> 24) as f32 * self.state.alpha) as u32;
            color = (a << 24) | (color & 0x00FF_FFFF);
        }
        // The draft carries blur ON the paint — no fused shadow verb.
        let mut p = base_paint(color, wtypes::PaintStyle::Fill, 0.0, 255);
        p.blur = (blur > 0.0).then_some(wtypes::MaskBlur {
            style: wtypes::BlurStyle::Normal,
            sigma: blur / 2.0,
        });
        self.canvas.draw_rounded_rect(wrrect(x, y, w, h, &radius, s), &p);
    }

    fn visit_opacity(
        &mut self,
        opacity_item: core::pin::Pin<&i_slint_core::items::Opacity>,
        _self_rc: &ItemRc,
        _size: LogicalSize,
    ) -> i_slint_core::items::RenderingResult {
        let opacity = opacity_item.opacity().clamp(0.0, 1.0);
        if opacity < 1.0 {
            self.canvas.save_layer(None, (opacity * 255.0) as u8);
            self.state.extra_canvas_saves += 1;
        }
        i_slint_core::items::RenderingResult::ContinueRenderingChildren
    }

    fn combine_clip(
        &mut self,
        rect: LogicalRect,
        radius: LogicalBorderRadius,
        border_width: LogicalLength,
    ) -> bool {
        let s = self.scale;
        // Shrink by the border width so children clip to the inner edge.
        let bw = border_width.get();
        let clip_rect = LogicalRect::new(
            LogicalPoint::new(rect.origin.x + bw, rect.origin.y + bw),
            LogicalSize::new(
                (rect.size.width - 2.0 * bw).max(0.0),
                (rect.size.height - 2.0 * bw).max(0.0),
            ),
        );
        let has_radius = radius.top_left.max(radius.top_right)
            .max(radius.bottom_right)
            .max(radius.bottom_left)
            > 0.0;
        if has_radius {
            self.canvas.clip_rounded_rect(
                wrrect(
                    clip_rect.origin.x * s,
                    clip_rect.origin.y * s,
                    clip_rect.size.width * s,
                    clip_rect.size.height * s,
                    &radius,
                    s,
                ),
                true,
            );
        } else {
            self.canvas.clip_rect(
                wrect(
                    clip_rect.origin.x * s,
                    clip_rect.origin.y * s,
                    clip_rect.size.width * s,
                    clip_rect.size.height * s,
                ),
                true,
            );
        }
        self.state.clip = self
            .state
            .clip
            .intersection(&clip_rect)
            .unwrap_or_else(|| LogicalRect::new(LogicalPoint::default(), LogicalSize::default()));
        !self.state.clip.is_empty()
    }

    fn get_current_clip(&self) -> LogicalRect {
        self.state.clip
    }

    fn translate(&mut self, distance: LogicalVector) {
        self.canvas.translate(distance.x * self.scale, distance.y * self.scale);
        self.state.clip = self.state.clip.translate(-distance);
    }

    fn rotate(&mut self, angle_in_degrees: f32) {
        self.canvas.rotate(angle_in_degrees);
        self.state.clip = unbounded_clip();
    }

    fn scale(&mut self, scale_x_factor: f32, scale_y_factor: f32) {
        self.canvas.scale(scale_x_factor, scale_y_factor);
        if scale_x_factor != 0.0 && scale_y_factor != 0.0 {
            let c = self.state.clip;
            self.state.clip = LogicalRect::new(
                LogicalPoint::new(c.origin.x / scale_x_factor, c.origin.y / scale_y_factor),
                LogicalSize::new(c.size.width / scale_x_factor, c.size.height / scale_y_factor),
            );
        }
    }

    fn apply_opacity(&mut self, opacity: f32) {
        self.state.alpha *= opacity.clamp(0.0, 1.0);
    }

    fn save_state(&mut self) {
        self.canvas.save();
        self.saved.push(self.state.clone());
        self.state.extra_canvas_saves = 0;
    }

    fn restore_state(&mut self) {
        for _ in 0..self.state.extra_canvas_saves {
            self.canvas.restore();
        }
        self.canvas.restore();
        if let Some(state) = self.saved.pop() {
            self.state = state;
        }
    }

    fn scale_factor(&self) -> f32 {
        self.scale
    }

    fn draw_cached_pixmap(
        &mut self,
        _item_cache: &ItemRc,
        update_fn: &dyn Fn(&mut dyn FnMut(u32, u32, &[u8])),
    ) {
        // Immediate-mode: upload + draw + drop each frame (premultiplied
        // RGBA → unpremultiply for image-from-rgba8).
        let alpha = (self.state.alpha * 255.0) as u8;
        let canvas = self.canvas;
        let graphics = self.graphics;
        update_fn(&mut |width, height, data| {
            if width == 0 || height == 0 {
                return;
            }
            let mut rgba = Vec::with_capacity(data.len());
            for px in data.chunks_exact(4) {
                let a = px[3];
                if a == 0 || a == 255 {
                    rgba.extend_from_slice(px);
                } else {
                    let un = |v: u8| ((v as u32 * 255) / a as u32).min(255) as u8;
                    rgba.extend_from_slice(&[un(px[0]), un(px[1]), un(px[2]), a]);
                }
            }
            if let Ok(img) = graphics.image_from_rgba8(width, height, &rgba) {
                let p = base_paint(0xFFFF_FFFF, wtypes::PaintStyle::Fill, 0.0, alpha);
                canvas.draw_image(&img, point(0.0, 0.0), linear_sampling(), &p);
                // img drops here → host resource freed.
            }
        });
    }

    fn draw_string(&mut self, string: &str, _color: i_slint_core::Color) {
        eprintln!("slint: {string}");
    }

    fn draw_image_direct(&mut self, image: i_slint_core::graphics::Image) {
        let size = image.size();
        if size.is_empty() {
            return;
        }
        let alpha = (self.state.alpha * 255.0) as u8;
        let canvas = self.canvas;
        self.with_image(&image, (size.width, size.height), |img| {
            let p = base_paint(0xFFFF_FFFF, wtypes::PaintStyle::Fill, 0.0, alpha);
            canvas.draw_image(img, point(0.0, 0.0), linear_sampling(), &p);
        });
    }

    fn window(&self) -> &WindowInner {
        WindowInner::from_pub(self.window)
    }

    fn as_any(&mut self) -> Option<&mut dyn core::any::Any> {
        None
    }
}

/// Glyph-level text (the draft's `glyphs` interface): parley hands font
/// blobs + positioned glyph ids in PHYSICAL units; the blob is registered
/// once (`typeface.from-bytes`) and runs forward to `draw-glyphs`.
impl GlyphRenderer for WandrItemRenderer<'_> {
    type PlatformBrush = TextBrush;

    fn platform_text_fill_brush(
        &mut self,
        brush: Brush,
        _size: LogicalSize,
    ) -> Option<Self::PlatformBrush> {
        if brush.is_transparent() {
            return None;
        }
        Some(TextBrush { color: color_argb(brush.color()), stroke_width: 0.0, stroke: false })
    }

    fn platform_brush_for_color(
        &mut self,
        color: &i_slint_core::Color,
    ) -> Option<Self::PlatformBrush> {
        if color.alpha() == 0 {
            return None;
        }
        Some(TextBrush { color: color_argb(*color), stroke_width: 0.0, stroke: false })
    }

    fn platform_text_stroke_brush(
        &mut self,
        brush: Brush,
        physical_stroke_width: f32,
        _size: LogicalSize,
    ) -> Option<Self::PlatformBrush> {
        if brush.is_transparent() {
            return None;
        }
        Some(TextBrush {
            color: color_argb(brush.color()),
            stroke_width: physical_stroke_width,
            stroke: true,
        })
    }

    fn draw_glyph_run(
        &mut self,
        font: &sharedparley::parley::FontData,
        font_size: sharedparley::PhysicalLength,
        _normalized_coords: &[i16],
        _synthesis: &fontique::Synthesis,
        brush: Self::PlatformBrush,
        y_offset: sharedparley::PhysicalLength,
        glyphs_it: &mut dyn Iterator<Item = sharedparley::parley::layout::Glyph>,
    ) {
        let glyphs: Vec<wglyphs::PositionedGlyph> = glyphs_it
            .map(|g| wglyphs::PositionedGlyph {
                id: g.id,
                at: point(g.x, g.y + y_offset.get()),
            })
            .collect();
        if glyphs.is_empty() {
            return;
        }
        let mut p = base_paint(
            brush.color,
            if brush.stroke { wtypes::PaintStyle::Stroke } else { wtypes::PaintStyle::Fill },
            brush.stroke_width,
            (self.state.alpha * 255.0) as u8,
        );
        if brush.stroke {
            p.stroke_miter = 10.0;
        }
        let canvas = self.canvas;
        with_typeface(font, |tf| {
            wglyphs::draw_glyphs(canvas, tf, font_size.get(), &glyphs, point(0.0, 0.0), &p);
        });
    }

    fn fill_rectangle(
        &mut self,
        physical_rect: sharedparley::PhysicalRect,
        brush: Self::PlatformBrush,
    ) {
        let p = base_paint(
            brush.color,
            wtypes::PaintStyle::Fill,
            0.0,
            (self.state.alpha * 255.0) as u8,
        );
        self.canvas.draw_rect(
            wrect(
                physical_rect.min_x(),
                physical_rect.min_y(),
                physical_rect.width(),
                physical_rect.height(),
            ),
            &p,
        );
    }
}
