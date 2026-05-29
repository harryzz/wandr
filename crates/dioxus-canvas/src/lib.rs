//! dioxus-canvas — a "tiny Blitz" that drives the wart canvas WIT from a dioxus
//! `VirtualDom` (task 59). The reactive, flexbox-laid-out alternative to a
//! hand-rolled canvas painter (`war.launcher`) for richer Rust guests, without
//! Kotlin/Compose's binary size / leak / working-set cost.
//!
//! Pipeline (the four steps from the spike README):
//!   1. VirtualDom mutations → retained node arena (`dom`, a `WriteMutations`
//!      stack machine over dioxus templates).
//!   2. node tags/styles → `taffy::Style`; `compute_layout` (text leaves measured
//!      via the host `measure-text` WIT verb through `CanvasSink`).
//!   3. walk the laid-out tree → canvas-WIT draw verbs (`begin_frame` / `clear` /
//!      `fill_rrect` / `create_text_blob` + `draw_text_blob`).
//!   4. hit-test pointer events → dioxus `handle_event` → re-render.
//!
//! Incremental like the launcher: re-diff + relayout only when the VirtualDom
//! reports mutations or input fired; otherwise replay the cached draw list.

mod dom;
mod events;
mod sink;
mod style;

use std::collections::HashMap;

use dioxus_core::{ElementId, VirtualDom};
use taffy::{AvailableSpace, Dimension, Display, FlexDirection, Size, Style, TaffyTree};

use dom::{Dom, NodeId, NodeKind};
use style::PaintProps;

pub use sink::{CanvasSink, Fill};

/// Per-text-leaf measurement context handed to taffy.
struct TextCtx {
    text: String,
    family: String,
    size: f32,
    weight: u32,
    italic: bool,
}

/// One cached paint command, in absolute device pixels.
enum DrawOp {
    Rrect { x: f32, y: f32, w: f32, h: f32, r: f32, color: u32 },
    Text { blob: u32, x: f32, y: f32, color: u32 },
}

// Pointer/key-listener flags (which dioxus events an element subscribes to).
const F_CLICK: u8 = 1;
const F_DOWN: u8 = 2;
const F_MOVE: u8 = 4;
const F_KEY: u8 = 8; // listens for keydown → focusable text input

struct HitRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    eid: u32,
    flags: u8,
}

/// In-progress non-slider gesture (tap vs. scroll). A press that isn't on a
/// draggable element starts here; on move past a threshold it becomes a scroll,
/// otherwise on release it's a tap → `click`.
struct Down {
    start_y: f32,
    last_y: f32,
    content_y: f32,
    click_eid: Option<u32>,
    scrolling: bool,
}

/// Movement (px) past which a press becomes a scroll instead of a tap.
const SCROLL_THRESHOLD: f32 = 16.0;

/// The renderer. Construct with the guest's root component, then drive it from
/// the `renderer` WIT export (`render_frame` / `on_pointer_down` / `on_resize`).
pub struct DomRenderer {
    vdom: VirtualDom,
    dom: Dom,
    /// Set when the VirtualDom may have pending mutations (initial build + after
    /// each dispatched event). Triggers a re-diff + relayout next frame.
    dirty: bool,
    surface: (f32, f32),
    draw_ops: Vec<DrawOp>,
    hits: Vec<HitRect>,
    /// Host blob ids from the current layout, dropped on relayout.
    /// Element capturing an in-progress drag: `(eid, rect x, y, w, h)`. Set on
    /// pointer-down over a draggable element; routes moves/ups to it.
    captured: Option<(u32, f32, f32, f32, f32)>,
    /// The focused text-input element (listens for keydown); `on_key` routes
    /// host key events here.
    focused: Option<u32>,
    /// UI scale factor. Guest styles are authored in logical px; the layout +
    /// paint multiply lengths/fonts by this, and input element-relative coords
    /// are divided by it (so guest coordinate math stays in logical px).
    scale: f32,
    /// Vertical scroll offset (physical px) + total laid-out content height.
    /// Content is laid out at natural height; the viewport scrolls. Applied at
    /// paint-replay + hit-test time — scrolling needs no relayout.
    scroll_y: f32,
    content_h: f32,
    /// In-progress tap/scroll gesture (non-slider press).
    down: Option<Down>,
    blobs: Vec<u32>,
    /// `(text, family, size-bits, weight, italic)` → `(w, h)`. Avoids a host
    /// round-trip per text leaf per layout (taffy measures repeatedly).
    measure_cache: HashMap<(String, String, u32, u32, bool), (f32, f32)>,
}

impl DomRenderer {
    pub fn new(app: fn() -> dioxus_core::Element) -> Self {
        events::install();
        DomRenderer {
            vdom: VirtualDom::new(app),
            dom: Dom::new(),
            dirty: true,
            surface: (0.0, 0.0),
            draw_ops: Vec::new(),
            hits: Vec::new(),
            captured: None,
            focused: None,
            scale: 1.0,
            scroll_y: 0.0,
            content_h: 0.0,
            down: None,
            blobs: Vec::new(),
            measure_cache: HashMap::new(),
        }
    }

    pub fn on_resize(&mut self, w: f32, h: f32) {
        self.surface = (w, h);
        self.dirty = true;
    }

    /// Set the UI scale factor (1.0 = author px). Triggers a relayout if changed.
    /// The guest can drive this at runtime (e.g. +/- buttons) by calling it each
    /// frame from `render_frame` with a value it owns.
    pub fn set_scale(&mut self, scale: f32) {
        let scale = scale.clamp(0.5, 4.0);
        if (scale - self.scale).abs() > f32::EPSILON {
            self.scale = scale;
            self.dirty = true;
        }
    }

    /// Paint a frame. Re-diffs + relayouts only when dirty, then replays the
    /// cached draw list every frame (cheap; mirrors the launcher).
    pub fn render_frame<S: CanvasSink>(&mut self, sink: &mut S) {
        if self.surface.0 == 0.0 {
            let (w, h) = sink.surface_size();
            self.surface = (w, h);
        }
        if self.dirty {
            // Apply the initial build the first time, incremental diffs after.
            if self.blobs.is_empty() && self.dom.root_children().is_empty() {
                self.vdom.rebuild(&mut self.dom);
            } else {
                self.vdom.process_events();
                self.vdom.render_immediate(&mut self.dom);
            }
            self.relayout(sink);
            self.dirty = false;
        }

        // Draw ops are in content space; the viewport scrolls by subtracting
        // scroll_y. Off-viewport content is clipped by the surface bounds.
        let sy = self.scroll_y;
        sink.begin_frame();
        sink.clear(0xFF12121A);
        for op in &self.draw_ops {
            match *op {
                DrawOp::Rrect { x, y, w, h, r, color } => {
                    sink.fill_rrect(x, y - sy, w, h, r, r, Fill { color });
                }
                DrawOp::Text { blob, x, y, color } => {
                    sink.draw_text_blob(blob, x, y - sy, Fill { color });
                }
            }
        }
        sink.end_frame();
    }

    /// Pointer down → hit-test the cached layout (top-most clickable rect under
    /// the point) → dispatch a dioxus `click` → mark dirty for re-render.
    /// Dispatch a dioxus event to an element. `(x,y)` are surface-absolute;
    /// `(ex,ey)` are relative to the element's rect (what sliders/pickers read
    /// via `event.element_coordinates()`).
    fn dispatch(&self, name: &str, eid: u32, x: f32, y: f32, ex: f32, ey: f32) {
        // Element-relative coords go back to the guest in LOGICAL px (the guest
        // authors + does its slider/picker math in logical units).
        let (ex, ey) = (ex / self.scale, ey / self.scale);
        #[allow(deprecated)]
        self.vdom
            .handle_event(name, events::mouse_event(x, y, ex, ey), ElementId(eid as usize), true);
    }

    /// Hit-test in content space (incoming `y` is viewport; add `scroll_y`).
    fn hit_at(&self, x: f32, content_y: f32) -> Option<(u32, u8, f32, f32, f32, f32)> {
        self.hits
            .iter()
            .rev()
            .find(|h| x >= h.x && x <= h.x + h.w && content_y >= h.y && content_y <= h.y + h.h)
            .map(|h| (h.eid, h.flags, h.x, h.y, h.w, h.h))
    }

    fn max_scroll(&self) -> f32 {
        (self.content_h - self.surface.1).max(0.0)
    }

    pub fn on_pointer_down(&mut self, x: f32, y: f32) {
        let cy = y + self.scroll_y; // content-space y
        let hit = self.hit_at(x, cy);
        match hit {
            // A draggable element (slider/picker) captures the pointer — no
            // scroll/tap gesture; drag goes straight to it.
            Some((eid, flags, hx, hy, hw, hh)) if flags & F_MOVE != 0 => {
                self.captured = Some((eid, hx, hy, hw, hh));
                if flags & F_DOWN != 0 {
                    self.dispatch("mousedown", eid, x, y, x - hx, cy - hy);
                }
                self.focused = None;
                self.dirty = true;
            }
            // Otherwise begin a tap/scroll gesture; defer click to release.
            _ => {
                let (click_eid, focus) = match hit {
                    Some((eid, flags, ..)) => (
                        if flags & F_CLICK != 0 { Some(eid) } else { None },
                        if flags & F_KEY != 0 { Some(eid) } else { None },
                    ),
                    None => (None, None),
                };
                self.focused = focus;
                self.down = Some(Down { start_y: y, last_y: y, content_y: cy, click_eid, scrolling: false });
                if focus.is_some() {
                    self.dirty = true;
                }
            }
        }
    }

    /// Route a host key event (`on-key-event-v2`) to the focused text input as a
    /// dioxus `keydown`/`keyup`. `code_point` is the Unicode scalar (0 for
    /// non-printable); `key_id` is a Compose-Key id (8=Backspace, 13=Enter, …).
    pub fn on_key(&mut self, down: bool, code_point: u32, key_id: u32) {
        if let Some(eid) = self.focused {
            let name = if down { "keydown" } else { "keyup" };
            #[allow(deprecated)]
            self.vdom.handle_event(
                name,
                events::key_event(code_point, key_id),
                ElementId(eid as usize),
                true,
            );
            self.dirty = true;
        }
    }

    /// Whether a text input currently has focus (the guest uses this to decide
    /// whether to keep the soft keyboard attached).
    pub fn has_focus(&self) -> bool {
        self.focused.is_some()
    }

    pub fn on_pointer_move(&mut self, x: f32, y: f32) {
        if let Some((eid, hx, hy, hw, hh)) = self.captured {
            // Slider/picker drag (content-space, clamped to the element box).
            let ex = (x - hx).clamp(0.0, hw);
            let ey = ((y + self.scroll_y) - hy).clamp(0.0, hh);
            self.dispatch("mousemove", eid, x, y, ex, ey);
            self.dirty = true;
            return;
        }
        // Otherwise: tap → scroll once the press moves past the threshold.
        let max = self.max_scroll();
        if let Some(d) = self.down.as_mut() {
            if !d.scrolling && (y - d.start_y).abs() > SCROLL_THRESHOLD {
                d.scrolling = true;
            }
            if d.scrolling {
                let dy = y - d.last_y;
                d.last_y = y;
                self.scroll_y = (self.scroll_y - dy).clamp(0.0, max);
                self.dirty = true;
            }
        }
    }

    pub fn on_pointer_up(&mut self, x: f32, y: f32) {
        if let Some((eid, hx, hy, hw, hh)) = self.captured.take() {
            let ex = (x - hx).clamp(0.0, hw);
            let ey = ((y + self.scroll_y) - hy).clamp(0.0, hh);
            self.dispatch("mouseup", eid, x, y, ex, ey);
            self.dirty = true;
            return;
        }
        // A press that didn't become a scroll is a tap → click on release.
        if let Some(d) = self.down.take() {
            if !d.scrolling {
                if let Some(eid) = d.click_eid {
                    self.dispatch("click", eid, x, d.content_y, 0.0, 0.0);
                    self.dirty = true;
                }
            }
        }
    }

    /// Centre of the first clickable rect. Exposed (doc-hidden) so tests can
    /// click it without reaching into private layout state.
    #[doc(hidden)]
    pub fn first_hit_center(&self) -> Option<(f32, f32)> {
        self.hits.first().map(|h| (h.x + h.w / 2.0, h.y + h.h / 2.0))
    }

    // ── Layout + paint ────────────────────────────────────────────────────

    fn relayout<S: CanvasSink>(&mut self, sink: &mut S) {
        // Drop the previous frame's host blobs.
        for id in self.blobs.drain(..) {
            sink.drop_text_blob(id);
        }
        self.draw_ops.clear();
        self.hits.clear();

        let mut taffy: TaffyTree<TextCtx> = TaffyTree::new();
        let mut map: HashMap<NodeId, taffy::NodeId> = HashMap::new();

        // A flex-column root sized to the surface; the app's roots are children.
        let root_kids: Vec<taffy::NodeId> = self
            .dom
            .root_children()
            .to_vec()
            .iter()
            .map(|&n| self.build_taffy(&mut taffy, &mut map, n, &PaintProps::default(), sink))
            .collect();
        // Root: full surface width, NATURAL height (content may exceed the
        // surface → it scrolls). Height left auto so the column takes its
        // content height; available height is MaxContent (unbounded).
        let root_style = Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            size: Size {
                width: Dimension::Length(self.surface.0),
                height: Dimension::Auto,
            },
            ..Default::default()
        };
        let root = taffy.new_with_children(root_style, &root_kids).unwrap();

        {
            // Bind disjoint fields so the measure closure can borrow the cache
            // while `taffy` is borrowed for the compute call.
            let cache = &mut self.measure_cache;
            taffy
                .compute_layout_with_measure(
                    root,
                    Size {
                        width: AvailableSpace::Definite(self.surface.0),
                        height: AvailableSpace::Definite(self.surface.1),
                    },
                    |known, _avail, _id, ctx, _style| match ctx {
                        Some(t) => {
                            let (w, h) = measure_cached(cache, sink, t);
                            Size {
                                width: known.width.unwrap_or(w),
                                height: known.height.unwrap_or(h),
                            }
                        }
                        None => Size::ZERO,
                    },
                )
                .unwrap();
        }

        // Record total content height + re-clamp the scroll offset to it.
        self.content_h = taffy.layout(root).map(|l| l.size.height).unwrap_or(self.surface.1);
        self.scroll_y = self.scroll_y.clamp(0.0, self.max_scroll());

        // Walk the laid-out tree (absolute coords) → draw ops + hit rects.
        for &n in &self.dom.root_children().to_vec() {
            self.paint_walk(&taffy, &map, n, 0.0, 0.0, &PaintProps::default(), sink);
        }
    }

    /// Recursively build the taffy tree mirroring the dom arena, carrying
    /// inherited font/colour props down to text leaves for measurement.
    fn build_taffy<S: CanvasSink>(
        &self,
        taffy: &mut TaffyTree<TextCtx>,
        map: &mut HashMap<NodeId, taffy::NodeId>,
        node: NodeId,
        inherited: &PaintProps,
        sink: &mut S,
    ) -> taffy::NodeId {
        let n = self.dom.node(node);
        let tid = match &n.kind {
            NodeKind::Element { style, .. } => {
                let paint = style::to_paint(style, inherited);
                let tstyle = style::to_taffy(style, self.scale);
                let kids: Vec<taffy::NodeId> = n
                    .children
                    .iter()
                    .map(|&c| self.build_taffy(taffy, map, c, &paint, sink))
                    .collect();
                taffy.new_with_children(tstyle, &kids).unwrap()
            }
            NodeKind::Text(text) => taffy
                .new_leaf_with_context(
                    Style::DEFAULT,
                    TextCtx {
                        text: text.clone(),
                        family: "sans-serif".to_string(),
                        size: inherited.font_size * self.scale,
                        weight: inherited.font_weight,
                        italic: inherited.italic,
                    },
                )
                .unwrap(),
            NodeKind::Placeholder => taffy.new_leaf(Style::DEFAULT).unwrap(),
        };
        let _ = sink;
        map.insert(node, tid);
        tid
    }

    fn paint_walk<S: CanvasSink>(
        &mut self,
        taffy: &TaffyTree<TextCtx>,
        map: &HashMap<NodeId, taffy::NodeId>,
        node: NodeId,
        parent_x: f32,
        parent_y: f32,
        inherited: &PaintProps,
        sink: &mut S,
    ) {
        let Some(&tid) = map.get(&node) else { return };
        let layout = taffy.layout(tid).unwrap();
        let x = parent_x + layout.location.x;
        let y = parent_y + layout.location.y;
        let (w, h) = (layout.size.width, layout.size.height);

        // Clone what we need before borrowing self mutably for draw ops.
        let n = self.dom.node(node);
        let kind_is_text = matches!(n.kind, NodeKind::Text(_));
        let (paint, children, eid, text) = match &n.kind {
            NodeKind::Element { style, listeners, .. } => {
                let p = style::to_paint(style, inherited);
                let mut flags = 0u8;
                for l in listeners {
                    match l.as_str() {
                        "click" => flags |= F_CLICK,
                        "mousedown" => flags |= F_DOWN,
                        "mousemove" => flags |= F_MOVE,
                        "keydown" => flags |= F_KEY,
                        _ => {}
                    }
                }
                let eid = if flags != 0 { n.element_id } else { None };
                (p, n.children.clone(), eid.map(|e| (e, flags)), None)
            }
            NodeKind::Text(t) => (*inherited, Vec::new(), None, Some(t.clone())),
            NodeKind::Placeholder => (*inherited, Vec::new(), None, None),
        };

        if !kind_is_text {
            if paint.background != 0 {
                // Negative radius = percent-of-min-dimension (e.g. 50% → circle/pill).
                let r = if paint.radius < 0.0 { w.min(h) * -paint.radius } else { paint.radius * self.scale };
                self.draw_ops.push(DrawOp::Rrect { x, y, w, h, r, color: paint.background });
            }
            if let Some((eid, flags)) = eid {
                self.hits.push(HitRect { x, y, w, h, eid, flags });
            }
        } else if let Some(t) = text {
            if !t.trim().is_empty() {
                let fs = paint.font_size * self.scale;
                let blob = sink.create_text_blob(&t, "sans-serif", fs, paint.font_weight, paint.italic);
                self.blobs.push(blob);
                // Baseline ≈ top + cap height. taffy gave the leaf its measured
                // box; sit the baseline near the bottom of that box.
                let baseline = y + fs;
                self.draw_ops.push(DrawOp::Text { blob, x, y: baseline, color: paint.color });
            }
        }

        for c in children {
            self.paint_walk(taffy, map, c, x, y, &paint, sink);
        }
    }
}

/// Measure a text run, caching by content+font so repeated taffy measure calls
/// and unchanged-across-frames text don't re-cross the WIT boundary.
fn measure_cached<S: CanvasSink>(
    cache: &mut HashMap<(String, String, u32, u32, bool), (f32, f32)>,
    sink: &mut S,
    t: &TextCtx,
) -> (f32, f32) {
    let key = (t.text.clone(), t.family.clone(), t.size.to_bits(), t.weight, t.italic);
    if let Some(&v) = cache.get(&key) {
        return v;
    }
    let v = sink.measure_text(&t.text, &t.family, t.size, t.weight, t.italic);
    cache.insert(key, v);
    v
}
