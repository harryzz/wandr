//! wart-arbiter-core — the arbiter kernel (task 73).
//!
//! The arbiter is being re-centralized into the native equivalent of
//! Android's `system_server` coordinator (WMS·IMMS·AMS): *it decides, it
//! never renders.* Per `docs/visual-sizing-design-patterns.md`, it grows by
//! **adding a crate and wiring one line**, never by digging into working
//! code (Open/Closed). This crate is the thin kernel every responsibility
//! module plugs into. It owns exactly four things:
//!
//!   1. a typed, **per-display** [`Store`] — the single source of truth;
//!   2. an [`Event`] vocabulary + the cascade that fans events to modules;
//!   3. the [`ArbiterModule`] trait;
//!   4. the [`Registry`] that routes verbs + events to modules.
//!
//! It owns **no surface, no socket, and never paints**. Modules never call
//! each other: they `emit` events and read/write the [`Store`] through
//! [`Ctx`], and queue host pushes via [`Ctx::deliver_to_host`]. The binary
//! owns the transport and flushes the queued pushes after a handler returns.
//!
//! Design effort belongs **here** (the event vocabulary, the per-display
//! state schema, the trait); get those right and responsibilities are
//! additive crates forever. So the full [`Event`] vocabulary is declared up
//! front even though the first module only fires a few of them.

use std::collections::HashMap;

/// Identifies one physical display. `0` is the primary panel. Keyed
/// per-display from day one — "one panel" is a hardcode the doc calls out
/// (DisplayManager is a foreseen responsibility), so all geometry hangs off
/// a `DisplayId` even while there is exactly one.
pub type DisplayId = u32;

/// The primary (and currently only) display.
pub const PRIMARY_DISPLAY: DisplayId = 0;

/// Orientation sentinel meaning "the host keeps its own orientation" — used
/// in a pushed [`DisplayGeometry`] while the arbiter is **not yet** the
/// rotation authority for that surface (the byte-equivalent checkpoint, and
/// any host the WM module isn't fanning rotation out to). A real dihedral
/// code is `0..=7`; `255` is unambiguous.
pub const ORIENT_HOST_OWNED: u32 = 255;

/// Inset sentinel meaning "the host keeps its own (env-sourced) chrome
/// inset" — sent for `inset_top`/`inset_bottom` while the arbiter has not
/// yet been told the chrome heights (the byte-equivalent checkpoint). A real
/// inset is a small px count; `0xFFFF` can never be a real chrome thickness.
pub const INSET_HOST_OWNED: u32 = 0xFFFF;

/// Per-display physical facts + the chrome/keyboard/orientation **policy**
/// the arbiter authors. This is the WM module's slice of the [`Store`]; the
/// computed rects it pushes to hosts are derived from these fields.
///
/// All lengths are **physical px** in this increment (the arbiter authors the
/// inset/keyboard deltas it genuinely owns; the host still owns panel dims and
/// density, so a true-dp model is deferred — see the plan). `panel_w`,
/// `panel_h` and `density` are mirrored here for the WM's own math but remain
/// host-sourced until a panel/density report-up exists.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayGeometry {
    /// Native portrait panel width, px (host-sourced; 0 = unknown).
    pub panel_w: u32,
    /// Native portrait panel height, px (host-sourced; 0 = unknown).
    pub panel_h: u32,
    /// dp scale (`lcd_density / 160`). Host-sourced; 0.0 = unknown.
    pub density: f32,
    /// Dihedral orientation code `0..=7`, or [`ORIENT_HOST_OWNED`] while the
    /// host remains the rotation authority for this display.
    pub orientation: u32,
    /// Status-bar chrome reserved at the user-top edge, px.
    pub inset_top: u32,
    /// Taskbar chrome reserved at the user-bottom edge, px.
    pub inset_bottom: u32,
    /// Soft-keyboard occlusion, px (0 = no keyboard). Arbiter-owned (task 68).
    pub keyboard_px: u32,
}

impl Default for DisplayGeometry {
    fn default() -> Self {
        Self {
            panel_w: 0,
            panel_h: 0,
            density: 0.0,
            orientation: ORIENT_HOST_OWNED,
            inset_top: 0,
            inset_bottom: 0,
            keyboard_px: 0,
        }
    }
}

/// The single source of truth. In this increment it holds **only** the new
/// per-display geometry; the legacy role/overlay/IME state stays in the
/// binary's `state.rs` and migrates into the Store only when its own module
/// is built (strangler: migrate when changing, not before).
#[derive(Debug, Default)]
pub struct Store {
    displays: HashMap<DisplayId, DisplayGeometry>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a display's geometry, if it has been touched.
    pub fn display(&self, id: DisplayId) -> Option<&DisplayGeometry> {
        self.displays.get(&id)
    }

    /// Mutable access, inserting a [`DisplayGeometry::default`] the first time
    /// a display is referenced (`get_or_default`).
    pub fn display_mut(&mut self, id: DisplayId) -> &mut DisplayGeometry {
        self.displays.entry(id).or_default()
    }

    /// Every display id currently in the store.
    pub fn display_ids(&self) -> Vec<DisplayId> {
        self.displays.keys().copied().collect()
    }
}

/// The event vocabulary — the bus's whole alphabet. Declared in full now so
/// that adding a producer later is `+1 emit`, not a core edit. Modules react
/// to events in [`ArbiterModule::on_event`]; they never observe each other
/// directly.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A display became known to the arbiter.
    DisplayAdded { id: DisplayId },
    /// The host reported the real panel size + density (deferred producer).
    PanelMeasured { id: DisplayId, w: u32, h: u32, density: f32 },
    /// The decided orientation for a display changed (dihedral code).
    OrientationChanged { id: DisplayId, orient: u32 },
    /// Chrome insets for a display changed (px).
    InsetsChanged { id: DisplayId, top: u32, bottom: u32 },
    /// The soft keyboard's occlusion changed (px; 0 = hidden). Task 68 seam.
    ImeHeightChanged { id: DisplayId, px: u32 },
    /// The foreground app changed. Emitted from the legacy foreground cascade.
    ForegroundChanged { app_id: Option<String>, pid: Option<i32> },
    /// The focused editor changed (pid; `None` = no focus).
    EditorFocusChanged { pid: Option<i32> },
    /// A module recomputed a display's geometry (post-recompute notice).
    GeometryRecomputed { id: DisplayId },
}

/// A command's outcome. The binary renders this to one wire line; `Ok`/`Err`
/// become the `OK `/`ERR ` prefix the existing client protocol expects.
#[derive(Clone, Debug)]
pub enum Reply {
    Ok(String),
    Err(String),
}

impl Reply {
    pub fn ok(body: impl Into<String>) -> Self {
        Reply::Ok(body.into())
    }
    pub fn err(body: impl Into<String>) -> Self {
        Reply::Err(body.into())
    }
    /// The single wire line, matching the legacy handlers' `OK …` / `ERR …`.
    pub fn render(&self) -> String {
        match self {
            Reply::Ok(s) => format!("OK {s}"),
            Reply::Err(s) => format!("ERR {s}"),
        }
    }
}

/// The handle a module uses during a command or event reaction. It exposes
/// the [`Store`] and two outboxes — emitted events (fanned to every module to
/// a fixpoint) and host pushes (flushed by the binary after the handler
/// returns). A fresh `Ctx` is created per `on_command` / `on_event` call; its
/// outboxes are drained by the [`Registry`].
pub struct Ctx<'a> {
    pub store: &'a mut Store,
    events: Vec<Event>,
    pushes: Vec<(i32, String)>,
}

impl<'a> Ctx<'a> {
    fn new(store: &'a mut Store) -> Self {
        Self { store, events: Vec::new(), pushes: Vec::new() }
    }

    /// Emit an event for other modules to react to. Cascaded to a fixpoint by
    /// the [`Registry`] after the current handler returns.
    pub fn emit(&mut self, e: Event) {
        self.events.push(e);
    }

    /// Queue a one-line push to a host's control socket (`wart-host-<pid>`).
    /// The binary owns the actual socket I/O; it flushes the queue after the
    /// handler + its event cascade complete. `line` should already end with
    /// `\n` per the wire convention.
    pub fn deliver_to_host(&mut self, pid: i32, line: impl Into<String>) {
        self.pushes.push((pid, line.into()));
    }
}

/// A responsibility plugged into the arbiter. New responsibility = new module
/// = new verbs + new event reactions; existing modules untouched.
pub trait ArbiterModule: Send {
    /// The command verbs this module owns. Must be disjoint across modules
    /// (and from the legacy match — the binary probes modules first).
    fn verbs(&self) -> &[&'static str];

    /// Handle one of this module's verbs. May read/write the store, emit
    /// events, and queue host pushes via `ctx`.
    fn on_command(&mut self, verb: &str, args: &str, ctx: &mut Ctx) -> Reply;

    /// React to an event emitted by any module (or injected from a legacy
    /// handler). Default: ignore. May itself emit + push via `ctx`.
    fn on_event(&mut self, _ev: &Event, _ctx: &mut Ctx) {}
}

/// Routes verbs and events to registered modules. The binary keeps one of
/// these for the daemon's lifetime.
#[derive(Default)]
pub struct Registry {
    modules: Vec<Box<dyn ArbiterModule>>,
    verb_index: HashMap<&'static str, usize>,
}

/// Cascade depth cap — guards against a module pair that ping-pongs events.
const MAX_CASCADE_DEPTH: usize = 16;

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a module, indexing its verbs. Panics on a duplicate verb —
    /// that is a build-time wiring bug, not a runtime condition.
    pub fn register(&mut self, m: Box<dyn ArbiterModule>) {
        let idx = self.modules.len();
        for v in m.verbs() {
            if let Some(prev) = self.verb_index.insert(v, idx) {
                panic!(
                    "wart-arbiter-core: verb {v:?} registered by two modules (idx {prev} and {idx})"
                );
            }
        }
        self.modules.push(m);
    }

    /// Does any module own this verb? (Lets the binary decide module-vs-legacy
    /// without running anything.)
    pub fn owns(&self, verb: &str) -> bool {
        self.verb_index.contains_key(verb)
    }

    /// Dispatch a command to its owning module, then drain the event cascade
    /// it triggered. Returns the reply + every host push accumulated across
    /// the command and the cascade. `None` if no module owns `verb` (the
    /// binary then falls through to its legacy match).
    pub fn dispatch_command(
        &mut self,
        verb: &str,
        args: &str,
        store: &mut Store,
    ) -> Option<(Reply, Vec<(i32, String)>)> {
        let idx = *self.verb_index.get(verb)?;
        let (reply, events, pushes) = {
            let mut ctx = Ctx::new(store);
            let reply = self.modules[idx].on_command(verb, args, &mut ctx);
            (reply, ctx.events, ctx.pushes)
        };
        let all_pushes = self.drain_cascade(store, events, pushes);
        Some((reply, all_pushes))
    }

    /// Inject an event from outside the module system (a legacy handler that
    /// changed shared state the modules care about) and run the cascade.
    /// Returns every host push the reacting modules produced.
    pub fn dispatch_event(&mut self, ev: Event, store: &mut Store) -> Vec<(i32, String)> {
        self.drain_cascade(store, vec![ev], Vec::new())
    }

    /// Fan a batch of events to every module's `on_event`, looping over any
    /// events those reactions emit until none remain (fixpoint), accumulating
    /// all host pushes. Bounded by [`MAX_CASCADE_DEPTH`].
    fn drain_cascade(
        &mut self,
        store: &mut Store,
        mut events: Vec<Event>,
        mut pushes: Vec<(i32, String)>,
    ) -> Vec<(i32, String)> {
        let mut depth = 0usize;
        while !events.is_empty() {
            depth += 1;
            if depth > MAX_CASCADE_DEPTH {
                log::warn!(
                    "wart-arbiter-core: event cascade exceeded depth {MAX_CASCADE_DEPTH}; \
                     dropping {} pending event(s)",
                    events.len()
                );
                break;
            }
            let batch = std::mem::take(&mut events);
            for ev in &batch {
                for m in self.modules.iter_mut() {
                    let mut ctx = Ctx::new(store);
                    m.on_event(ev, &mut ctx);
                    events.extend(ctx.events);
                    pushes.extend(ctx.pushes);
                }
            }
        }
        pushes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Echo {
        seen: Vec<Event>,
    }
    impl ArbiterModule for Echo {
        fn verbs(&self) -> &[&'static str] {
            &["echo"]
        }
        fn on_command(&mut self, _verb: &str, args: &str, ctx: &mut Ctx) -> Reply {
            ctx.emit(Event::InsetsChanged { id: PRIMARY_DISPLAY, top: 1, bottom: 2 });
            ctx.deliver_to_host(42, format!("echo {args}\n"));
            Reply::ok(format!("echoed {args}"))
        }
        fn on_event(&mut self, ev: &Event, ctx: &mut Ctx) {
            self.seen.push(ev.clone());
            // React once to prove the cascade runs but terminates.
            if let Event::InsetsChanged { id, .. } = ev {
                ctx.deliver_to_host(7, "reacted\n");
                let _ = id;
            }
        }
    }

    #[test]
    fn command_dispatch_and_cascade() {
        let mut reg = Registry::new();
        reg.register(Box::new(Echo::default()));
        let mut store = Store::new();
        let (reply, pushes) = reg
            .dispatch_command("echo", "hi", &mut store)
            .expect("module owns echo");
        assert_eq!(reply.render(), "OK echoed hi");
        // The command push, plus the cascade reaction push.
        assert_eq!(pushes, vec![(42, "echo hi\n".to_string()), (7, "reacted\n".to_string())]);
    }

    #[test]
    fn unowned_verb_falls_through() {
        let mut reg = Registry::new();
        reg.register(Box::new(Echo::default()));
        let mut store = Store::new();
        assert!(reg.dispatch_command("launch", "x", &mut store).is_none());
        assert!(!reg.owns("launch"));
        assert!(reg.owns("echo"));
    }

    #[test]
    fn display_store_is_get_or_default() {
        let mut store = Store::new();
        assert!(store.display(PRIMARY_DISPLAY).is_none());
        store.display_mut(PRIMARY_DISPLAY).inset_top = 96;
        assert_eq!(store.display(PRIMARY_DISPLAY).unwrap().inset_top, 96);
    }
}
