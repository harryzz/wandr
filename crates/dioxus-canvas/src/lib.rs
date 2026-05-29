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

// Pointer-listener flags (which dioxus events an element subscribes to).
const F_CLICK: u8 = 1;
const F_DOWN: u8 = 2;
const F_MOVE: u8 = 4;

struct HitRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    eid: u32,
    flags: u8,
}

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
            blobs: Vec::new(),
            measure_cache: HashMap::new(),
        }
    }

    pub fn on_resize(&mut self, w: f32, h: f32) {
        self.surface = (w, h);
        self.dirty = true;
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

        sink.begin_frame();
        sink.clear(0xFF12121A);
        for op in &self.draw_ops {
            match *op {
                DrawOp::Rrect { x, y, w, h, r, color } => {
                    sink.fill_rrect(x, y, w, h, r, r, Fill { color });
                }
                DrawOp::Text { blob, x, y, color } => {
                    sink.draw_text_blob(blob, x, y, Fill { color });
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
        #[allow(deprecated)]
        self.vdom
            .handle_event(name, events::mouse_event(x, y, ex, ey), ElementId(eid as usize), true);
    }

    pub fn on_pointer_down(&mut self, x: f32, y: f32) {
        // Topmost pointer target under the point.
        let hit = self
            .hits
            .iter()
            .rev()
            .find(|h| x >= h.x && x <= h.x + h.w && y >= h.y && y <= h.y + h.h)
            .map(|h| (h.eid, h.flags, h.x, h.y, h.w, h.h));
        if let Some((eid, flags, hx, hy, hw, hh)) = hit {
            let (ex, ey) = (x - hx, y - hy);
            if flags & F_DOWN != 0 {
                self.dispatch("mousedown", eid, x, y, ex, ey);
            }
            if flags & F_CLICK != 0 {
                self.dispatch("click", eid, x, y, ex, ey);
            }
            // A draggable element (listens for move) captures the pointer.
            if flags & F_MOVE != 0 {
                self.captured = Some((eid, hx, hy, hw, hh));
            }
            self.dirty = true;
        }
    }

    pub fn on_pointer_move(&mut self, x: f32, y: f32) {
        if let Some((eid, hx, hy, hw, hh)) = self.captured {
            // Clamp the element-relative point to the element's box so a drag
            // past the edge still reports a sensible value.
            let ex = (x - hx).clamp(0.0, hw);
            let ey = (y - hy).clamp(0.0, hh);
            self.dispatch("mousemove", eid, x, y, ex, ey);
            self.dirty = true;
        }
    }

    pub fn on_pointer_up(&mut self, x: f32, y: f32) {
        if let Some((eid, hx, hy, hw, hh)) = self.captured.take() {
            let ex = (x - hx).clamp(0.0, hw);
            let ey = (y - hy).clamp(0.0, hh);
            self.dispatch("mouseup", eid, x, y, ex, ey);
            self.dirty = true;
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
        let root_style = Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            size: Size {
                width: Dimension::Length(self.surface.0),
                height: Dimension::Length(self.surface.1),
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
                let tstyle = style::to_taffy(style);
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
                        size: inherited.font_size,
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
                let r = if paint.radius < 0.0 { w.min(h) * -paint.radius } else { paint.radius };
                self.draw_ops.push(DrawOp::Rrect { x, y, w, h, r, color: paint.background });
            }
            if let Some((eid, flags)) = eid {
                self.hits.push(HitRect { x, y, w, h, eid, flags });
            }
        } else if let Some(t) = text {
            if !t.trim().is_empty() {
                let blob = sink.create_text_blob(&t, "sans-serif", paint.font_size, paint.font_weight, paint.italic);
                self.blobs.push(blob);
                // Baseline ≈ top + cap height. taffy gave the leaf its measured
                // box; sit the baseline near the bottom of that box.
                let baseline = y + paint.font_size;
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
