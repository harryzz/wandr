//! The `ItemRenderer` that draws Slint items through the `my:skiko-gfx`
//! canvas verbs. M1 skeleton (task 100): state management (transform/clip
//! tracking, save/restore, opacity) + solid fills, border rectangles and
//! box shadows are real; images (M2), gradients (M2), layers (M2) and text
//! (M3, `GlyphRenderer` → draw-glyphs) follow.
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

use i_slint_core::Brush;
use i_slint_core::items::ItemRc;
use i_slint_core::lengths::{
    LogicalBorderRadius, LogicalLength, LogicalPoint, LogicalRect, LogicalSize, LogicalVector,
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

#[derive(Clone)]
struct RenderState {
    alpha: f32,
    clip: LogicalRect,
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
            },
            saved: Vec::new(),
        }
    }

    /// Paint the window background under the item tree (called by the
    /// adapter before `render_component_items`).
    pub fn fill_window_background(&mut self, brush: Brush) {
        if brush.is_transparent() {
            return;
        }
        canvas::draw_paint(self.solid_paint(&brush));
    }

    /// M1 brush handling: solid colors are exact; gradients degrade to the
    /// brush's representative color (M2 maps them to create-*-gradient
    /// shaders by id).
    fn solid_paint(&self, brush: &Brush) -> canvas::PaintAttrs {
        let c = brush.color();
        paint_attrs(
            color_argb(c),
            canvas::PaintStyle::Fill,
            0.0,
            (self.state.alpha * 255.0) as u8,
        )
    }

    fn stroke_paint(&self, brush: &Brush, width_px: f32) -> canvas::PaintAttrs {
        let c = brush.color();
        paint_attrs(
            color_argb(c),
            canvas::PaintStyle::Stroke,
            width_px,
            (self.state.alpha * 255.0) as u8,
        )
    }
}

fn color_argb(c: i_slint_core::Color) -> u32 {
    ((c.alpha() as u32) << 24)
        | ((c.red() as u32) << 16)
        | ((c.green() as u32) << 8)
        | (c.blue() as u32)
}

fn paint_attrs(color: u32, style: canvas::PaintStyle, stroke_width: f32, alpha: u8) -> canvas::PaintAttrs {
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
/// the canvas clip/draw rrect verbs take one (rx, ry). Per-corner shapes
/// go through clip-path / an 8-float radii verb in M2.
fn uniform_radius(r: &LogicalBorderRadius) -> f32 {
    r.top_left.max(r.top_right).max(r.bottom_right).max(r.bottom_left).max(0.0)
}

impl i_slint_core::item_rendering::ItemRenderer for WandrItemRenderer<'_> {
    fn draw_rectangle(
        &mut self,
        rect: core::pin::Pin<&dyn i_slint_core::item_rendering::RenderRectangle>,
        _self_rc: &ItemRc,
        size: LogicalSize,
        _cache: &i_slint_core::item_rendering::CachedRenderingData,
    ) {
        let brush = rect.background();
        if brush.is_transparent() {
            return;
        }
        let s = self.scale;
        canvas::draw_rect(0.0, 0.0, size.width * s, size.height * s, self.solid_paint(&brush));
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

        let background = rect.background();
        if !background.is_transparent() {
            let p = self.solid_paint(&background);
            if radius > 0.0 {
                canvas::draw_rrect(0.0, 0.0, w, h, radius, radius, p);
            } else {
                canvas::draw_rect(0.0, 0.0, w, h, p);
            }
        }

        let border_width = rect.border_width().get() * s;
        let border_color = rect.border_color();
        if border_width > 0.0 && !border_color.is_transparent() {
            // Stroke centered on the inset midline so the border stays
            // inside the item bounds (Slint's border model).
            let inset = border_width / 2.0;
            let p = self.stroke_paint(&border_color, border_width);
            let r = (radius - inset).max(0.0);
            if r > 0.0 || radius > 0.0 {
                canvas::draw_rrect(
                    inset, inset, w - border_width, h - border_width, r, r, p,
                );
            } else {
                canvas::draw_rect(inset, inset, w - border_width, h - border_width, p);
            }
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
        _image: core::pin::Pin<&dyn i_slint_core::item_rendering::RenderImage>,
        _self_rc: &ItemRc,
        _size: LogicalSize,
        _cache: &i_slint_core::item_rendering::CachedRenderingData,
    ) {
        // M2: decode via create-image(-from-encoded), cache by Image cache
        // key, draw-image-rect with fit/alignment + colorize.
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
        _path: core::pin::Pin<&i_slint_core::items::Path>,
        _self_rc: &ItemRc,
        _size: LogicalSize,
    ) {
        // M2: lyon path events → SVG path string → canvas draw-path.
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
        // M1: alpha-multiply into subsequent paints (correct for leaf
        // content; M2 switches visit_opacity to a save-layer/bitmap-canvas
        // layer for correct group opacity).
        self.state.alpha *= opacity.clamp(0.0, 1.0);
    }

    fn save_state(&mut self) {
        canvas::save();
        self.saved.push(self.state.clone());
    }

    fn restore_state(&mut self) {
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
        _update_fn: &dyn Fn(&mut dyn FnMut(u32, u32, &[u8])),
    ) {
        // M2: create-image from the RGBA callback + draw-image (cached by
        // item like the upstream renderers).
    }

    fn draw_string(&mut self, string: &str, _color: i_slint_core::Color) {
        // Debug overlay only upstream; route to the host log for now.
        canvas::log_message(string);
    }

    fn draw_image_direct(&mut self, _image: i_slint_core::graphics::Image) {
        // M2 (drag-and-drop overlay image).
    }

    fn window(&self) -> &WindowInner {
        WindowInner::from_pub(self.window)
    }

    fn as_any(&mut self) -> Option<&mut dyn core::any::Any> {
        None
    }
}
