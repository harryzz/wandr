//! The `ItemRenderer` that draws Slint items through the `my:skiko-gfx`
//! canvas verbs. M2 (task 100): full geometry — gradient brushes (host
//! shader ids), images (render_to_buffer → create-image, `fit()` placement,
//! cached by ImageCacheKey), group opacity via save-layer, lyon paths as
//! SVG strings, cached pixmaps. Text is M3 (`GlyphRenderer` → draw-glyphs).
//!
//! Coordinates: Slint hands the renderer LOGICAL units and a scale factor;
//! the canvas draws in physical surface pixels, so every geometry value is
//! multiplied by `scale` at the call site (the upstream skia renderer's
//! convention — no global canvas scale, so stroke widths stay explicit).
//!
//! Clip tracking is guest-side (the femtovg pattern; the WIT deliberately
//! has no canvas state queries — docs/skia-wit-mapping.md). The tracked
//! rect lives in CURRENT local coordinates: translate/scale fold into it,
//! rotation invalidates it to "infinite" so `filter_item` culling stays
//! conservative (never wrongly culls).
//!
//! Known M2 approximations (carry to M4 visual review):
//! - per-corner border radii degrade to the max corner (uniform rrect);
//! - `colorize` maps to the host color-filter (Modulate): exact for the
//!   usual white/monochrome icons, a tint (not a replace) otherwise;
//! - Path fill-rule is always Winding (SVG strings carry no fill rule);
//! - image tiling falls back to a stretched draw.

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
use i_slint_core::window::WindowInner;

use crate::canvas;

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
    /// Host image ids by Slint image cache key (+ render size, which matters
    /// for scalable SVG sources). Images commonly outlive components, so
    /// this is global; bounded by the set of distinct images an app shows.
    static IMAGE_CACHE: RefCell<HashMap<(ImageCacheKey, u32, u32), u32>> =
        RefCell::new(HashMap::new());
}

#[derive(Clone)]
struct RenderState {
    alpha: f32,
    clip: LogicalRect,
    /// Canvas saves issued beyond the save_state() one (opacity layers),
    /// popped by the matching restore_state().
    extra_canvas_saves: u32,
}

pub struct WandrItemRenderer<'a> {
    window: &'a i_slint_core::api::Window,
    scale: f32,
    state: RenderState,
    saved: Vec<RenderState>,
}

impl<'a> WandrItemRenderer<'a> {
    pub fn new(window: &'a i_slint_core::api::Window, scale: f32) -> Self {
        let size = window.size();
        Self {
            window,
            scale,
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

    /// Paint the window background under the item tree (called by the
    /// adapter before `render_component_items`).
    pub fn fill_window_background(&mut self, brush: Brush) {
        let size = self.window.size();
        self.with_brush_paint(
            &brush,
            size.width as f32,
            size.height as f32,
            canvas::PaintStyle::Fill,
            0.0,
            |p| canvas::draw_paint(p),
        );
    }

    /// Build a paint for `brush` (gradients become host shader ids, created
    /// for the draw and dropped right after), hand it to `f`. No-op for
    /// transparent brushes. `w`/`h` are the brush geometry in PHYSICAL px.
    fn with_brush_paint(
        &self,
        brush: &Brush,
        w: f32,
        h: f32,
        style: canvas::PaintStyle,
        stroke_width: f32,
        f: impl FnOnce(canvas::PaintAttrs),
    ) {
        if brush.is_transparent() {
            return;
        }
        let shader = make_brush_shader(brush, w, h, self.scale);
        let mut p = paint_attrs(
            color_argb(brush.color()),
            style,
            stroke_width,
            (self.state.alpha * 255.0) as u8,
        );
        if let Some(id) = shader {
            p.shader_id = id;
        }
        f(p);
        if let Some(id) = shader {
            canvas::drop_shader(id);
        }
    }

    /// Resolve a Slint image to a live host image id (decode/render to a
    /// buffer once, then cached). `target` = physical render size hint for
    /// scalable sources (SVG).
    fn ensure_image(
        &self,
        image: &i_slint_core::graphics::Image,
        target: (u32, u32),
    ) -> Option<u32> {
        let inner: &i_slint_core::ImageInner = image.into();
        let key = ImageCacheKey::new(inner).map(|k| (k, target.0, target.1));
        if let Some(k) = &key {
            if let Some(id) = IMAGE_CACHE.with(|c| c.borrow().get(k).copied()) {
                return Some(id);
            }
        }
        let buffer = inner.render_to_buffer(Some(
            euclid::Size2D::new(target.0.max(1), target.1.max(1)),
        ))?;
        let (w, h, rgba) = buffer_to_rgba8_unpremul(&buffer);
        if w == 0 || h == 0 {
            return None;
        }
        let id = canvas::create_image(w, h, &rgba);
        if id == 0 {
            return None;
        }
        if let Some(k) = key {
            IMAGE_CACHE.with(|c| c.borrow_mut().insert(k, id));
        }
        Some(id)
    }
}

fn color_argb(c: i_slint_core::Color) -> u32 {
    ((c.alpha() as u32) << 24)
        | ((c.red() as u32) << 16)
        | ((c.green() as u32) << 8)
        | (c.blue() as u32)
}

/// Gradient brushes → host shader ids (Clamp tiling, like upstream).
/// Geometry follows the upstream skia renderer: linear via
/// `line_for_angle`, radial center/radius scaled, conic = full sweep with
/// 0° at 12 o'clock (the sweep verb's angles are degrees from 3 o'clock,
/// hence the -90 start).
fn make_brush_shader(brush: &Brush, w: f32, h: f32, scale: f32) -> Option<u32> {
    let id = match brush {
        Brush::LinearGradient(g) => {
            let (start, end) =
                i_slint_core::graphics::line_for_angle(g.angle(), [w, h].into());
            let (colors, positions): (Vec<u32>, Vec<f32>) =
                g.stops().map(|s| (color_argb(s.color), s.position)).unzip();
            canvas::create_linear_gradient(
                start.x, start.y, end.x, end.y, &colors, &positions, 0,
            )
        }
        Brush::RadialGradient(g) => {
            let (cx, cy) = g.center_or_default_scaled(w, h, scale);
            let radius = g.radius_or_default_scaled(w, h, scale);
            let (colors, positions): (Vec<u32>, Vec<f32>) =
                g.stops().map(|s| (color_argb(s.color), s.position)).unzip();
            canvas::create_radial_gradient(cx, cy, radius, &colors, &positions, 0)
        }
        Brush::ConicGradient(g) => {
            let (cx, cy) = g.center_or_default_scaled(w, h, scale);
            let (colors, positions): (Vec<u32>, Vec<f32>) =
                g.stops().map(|s| (color_argb(s.color), s.position)).unzip();
            canvas::create_sweep_gradient(cx, cy, -90.0, 270.0, &colors, &positions, 0)
        }
        _ => return None,
    };
    (id != 0).then_some(id)
}

fn paint_attrs(
    color: u32,
    style: canvas::PaintStyle,
    stroke_width: f32,
    alpha: u8,
) -> canvas::PaintAttrs {
    canvas::PaintAttrs {
        color,
        style,
        stroke_width,
        stroke_miter: 4.0,
        stroke_cap: canvas::StrokeCap::Butt,
        stroke_join: canvas::StrokeJoin::Miter,
        anti_alias: true,
        alpha,
        blend_mode: canvas::BlendMode::SrcOver,
        shader_id: 0,
        color_filter_kind: canvas::ColorFilterKind::None,
        color_filter_color: 0,
    }
}

/// Uniform-radius approximation of Slint's per-corner border radius —
/// the canvas clip/draw rrect verbs take one (rx, ry).
fn uniform_radius(r: &LogicalBorderRadius) -> f32 {
    r.top_left.max(r.top_right).max(r.bottom_right).max(r.bottom_left).max(0.0)
}

/// Any Slint pixel buffer → tightly-packed RGBA8 straight-alpha (what the
/// host `create-image` expects: RGBA8888 Unpremul).
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

/// lyon path events → SVG path string in PHYSICAL px (the canvas
/// `draw-path`/`clip-path` wire format).
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
        self.with_brush_paint(&rect.background(), w, h, canvas::PaintStyle::Fill, 0.0, |p| {
            canvas::draw_rect(0.0, 0.0, w, h, p);
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
        let radius = uniform_radius(&rect.border_radius()) * s;
        let (w, h) = (size.width * s, size.height * s);

        self.with_brush_paint(&rect.background(), w, h, canvas::PaintStyle::Fill, 0.0, |p| {
            if radius > 0.0 {
                canvas::draw_rrect(0.0, 0.0, w, h, radius, radius, p);
            } else {
                canvas::draw_rect(0.0, 0.0, w, h, p);
            }
        });

        let border_width = rect.border_width().get() * s;
        let border_color = rect.border_color();
        if border_width > 0.0 && !border_color.is_transparent() {
            // Stroke centered on the inset midline so the border stays
            // inside the item bounds (Slint's border model).
            let inset = border_width / 2.0;
            let r = (radius - inset).max(0.0);
            self.with_brush_paint(
                &border_color,
                w,
                h,
                canvas::PaintStyle::Stroke,
                border_width,
                |p| {
                    if r > 0.0 || radius > 0.0 {
                        canvas::draw_rrect(
                            inset, inset, w - border_width, h - border_width, r, r, p,
                        );
                    } else {
                        canvas::draw_rect(inset, inset, w - border_width, h - border_width, p);
                    }
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
            image.tiling(), // tiled output degrades to a stretched draw (M2)
        );
        let Some(img_id) =
            self.ensure_image(&source, (dest.width.ceil() as u32, dest.height.ceil() as u32))
        else {
            return;
        };
        // The host image may differ in size from source.size() (SVG rendered
        // at target size) — map the source clip proportionally.
        let (iw, ih) = (canvas::get_image_width(img_id), canvas::get_image_height(img_id));
        if iw == 0 || ih == 0 {
            return;
        }
        let sx = iw as f32 / source_size.width as f32;
        let sy = ih as f32 / source_size.height as f32;

        let mut p = paint_attrs(
            0xFFFF_FFFF,
            canvas::PaintStyle::Fill,
            0.0,
            (self.state.alpha * 255.0) as u8,
        );
        let colorize = image.colorize();
        if !colorize.is_transparent() {
            // Host color-filter = Modulate: exact for white/monochrome
            // icons (the common case), a tint for everything else.
            p.color_filter_kind = canvas::ColorFilterKind::Blend;
            p.color_filter_color = color_argb(colorize.color());
        }

        canvas::save();
        canvas::clip_rect(fit.offset.x, fit.offset.y, fit.size.width, fit.size.height, true);
        canvas::draw_image_rect(
            img_id,
            fit.clip_rect.origin.x as f32 * sx,
            fit.clip_rect.origin.y as f32 * sy,
            fit.clip_rect.size.width as f32 * sx,
            fit.clip_rect.size.height as f32 * sy,
            fit.offset.x,
            fit.offset.y,
            fit.clip_rect.size.width as f32 * fit.source_to_target_x,
            fit.clip_rect.size.height as f32 * fit.source_to_target_y,
            p,
        );
        canvas::restore();
    }

    fn draw_text(
        &mut self,
        _text: core::pin::Pin<&dyn i_slint_core::item_rendering::RenderText>,
        _self_rc: &ItemRc,
        _size: LogicalSize,
        _cache: &i_slint_core::item_rendering::CachedRenderingData,
    ) {
        // M3: sharedparley draw helpers through GlyphRenderer → draw-glyphs.
    }

    fn draw_text_input(
        &mut self,
        _text_input: core::pin::Pin<&i_slint_core::items::TextInput>,
        _self_rc: &ItemRc,
        _size: LogicalSize,
    ) {
        // M3 (with cursor/selection rects from sharedparley).
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
        let s = self.scale;
        canvas::save();
        canvas::translate(offset.x * s, offset.y * s);

        // Brush geometry: the path's viewbox-fitted bounds aren't tracked
        // here; use the item's viewbox size as the gradient frame.
        let (gw, gh) = (path.viewbox_width() * s, path.viewbox_height() * s);

        let fill = path.fill();
        if !fill.is_transparent() {
            self.with_brush_paint(&fill, gw, gh, canvas::PaintStyle::Fill, 0.0, |p| {
                canvas::draw_path(svg.as_bytes(), p);
            });
        }
        let stroke = path.stroke();
        let stroke_width = path.stroke_width().get() * s;
        if !stroke.is_transparent() && stroke_width > 0.0 {
            self.with_brush_paint(
                &stroke,
                gw,
                gh,
                canvas::PaintStyle::Stroke,
                stroke_width,
                |p| {
                    canvas::draw_path(svg.as_bytes(), p);
                },
            );
        }
        canvas::restore();
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
        // Slint blur maps to a Gaussian sigma of blur/2 (upstream skia
        // renderer convention).
        let radius = (uniform_radius(&box_shadow.logical_border_radius()) * s + spread).max(0.0);
        let mut color = color_argb(box_shadow.color());
        if self.state.alpha < 1.0 {
            let a = ((color >> 24) as f32 * self.state.alpha) as u32;
            color = (a << 24) | (color & 0x00FF_FFFF);
        }
        canvas::draw_shadow_rrect(x, y, w, h, &[radius], blur / 2.0, color);
    }

    fn visit_opacity(
        &mut self,
        opacity_item: core::pin::Pin<&i_slint_core::items::Opacity>,
        _self_rc: &ItemRc,
        _size: LogicalSize,
    ) -> i_slint_core::items::RenderingResult {
        let opacity = opacity_item.opacity().clamp(0.0, 1.0);
        if opacity < 1.0 {
            // Correct GROUP opacity: composite the subtree through a host
            // save-layer (unbounded; the host applies alpha at restore).
            canvas::save_layer(0.0, 0.0, 0.0, 0.0, false, (opacity * 255.0) as u8);
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
        // Shrink by the border width like the upstream renderers, so
        // children clip to the inner edge of a border.
        let bw = border_width.get();
        let clip_rect = LogicalRect::new(
            LogicalPoint::new(rect.origin.x + bw, rect.origin.y + bw),
            LogicalSize::new(
                (rect.size.width - 2.0 * bw).max(0.0),
                (rect.size.height - 2.0 * bw).max(0.0),
            ),
        );
        let r = uniform_radius(&radius) * s;
        if r > 0.0 {
            canvas::clip_rrect(
                clip_rect.origin.x * s,
                clip_rect.origin.y * s,
                clip_rect.size.width * s,
                clip_rect.size.height * s,
                r,
                r,
                true,
            );
        } else {
            canvas::clip_rect(
                clip_rect.origin.x * s,
                clip_rect.origin.y * s,
                clip_rect.size.width * s,
                clip_rect.size.height * s,
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
        canvas::translate(distance.x * self.scale, distance.y * self.scale);
        self.state.clip = self.state.clip.translate(-distance);
    }

    fn rotate(&mut self, angle_in_degrees: f32) {
        canvas::rotate(angle_in_degrees);
        // The tracked clip is no longer axis-aligned in local coords —
        // disable culling for the rest of this state (host clip stays exact).
        self.state.clip = unbounded_clip();
    }

    fn scale(&mut self, scale_x_factor: f32, scale_y_factor: f32) {
        canvas::scale(scale_x_factor, scale_y_factor);
        if scale_x_factor != 0.0 && scale_y_factor != 0.0 {
            let c = self.state.clip;
            self.state.clip = LogicalRect::new(
                LogicalPoint::new(c.origin.x / scale_x_factor, c.origin.y / scale_y_factor),
                LogicalSize::new(c.size.width / scale_x_factor, c.size.height / scale_y_factor),
            );
        }
    }

    fn apply_opacity(&mut self, opacity: f32) {
        // Per-paint alpha fold (used when something other than
        // visit_opacity applies opacity, e.g. semi-transparent layers).
        self.state.alpha *= opacity.clamp(0.0, 1.0);
    }

    fn save_state(&mut self) {
        canvas::save();
        self.saved.push(self.state.clone());
        self.state.extra_canvas_saves = 0;
    }

    fn restore_state(&mut self) {
        for _ in 0..self.state.extra_canvas_saves {
            canvas::restore();
        }
        canvas::restore();
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
        // Immediate-mode: upload + draw + drop each frame. The data is
        // premultiplied RGBA — unpremultiply for create-image (Unpremul).
        // TODO(M4): cache by item like the upstream renderers if profiling
        // shows icon-heavy UIs re-uploading every frame.
        let alpha = (self.state.alpha * 255.0) as u8;
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
            let id = canvas::create_image(width, height, &rgba);
            if id != 0 {
                canvas::draw_image(id, 0.0, 0.0, alpha);
                canvas::drop_image(id);
            }
        });
    }

    fn draw_string(&mut self, string: &str, _color: i_slint_core::Color) {
        // Debug overlay only upstream; route to the host log for now.
        canvas::log_message(string);
    }

    fn draw_image_direct(&mut self, image: i_slint_core::graphics::Image) {
        let size = image.size();
        if size.is_empty() {
            return;
        }
        let alpha = (self.state.alpha * 255.0) as u8;
        if let Some(id) = self.ensure_image(&image, (size.width, size.height)) {
            canvas::draw_image(id, 0.0, 0.0, alpha);
        }
    }

    fn window(&self) -> &WindowInner {
        WindowInner::from_pub(self.window)
    }

    fn as_any(&mut self) -> Option<&mut dyn core::any::Any> {
        None
    }
}
