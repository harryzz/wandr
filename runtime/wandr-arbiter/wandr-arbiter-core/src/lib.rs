//! wandr-arbiter-core — the arbiter kernel (task 73).
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

mod alarm;
mod notify;
mod registry;
mod surface;
pub use alarm::Alarm;
pub use notify::Notification;
pub use registry::{pid_alive, AppState, DEFAULT_IME_HEIGHT_PX};
pub use surface::{
    ChromeAnchor, DisplayState, EditorInfo, ResourceFocus, ResourceKind, Role, Surface,
};

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

// ── Chrome dimensions, in density-independent pixels (Arbiter Inc. 3b) ──
//
// The ONE named source of truth for the chrome heights (the no-hardcoding
// rule: a genuinely-needed constant lives in the layer that owns the policy —
// the arbiter). Physical px = dp × density (density reported up by the host).
// Back-derived from the tuned px at the Pixel 2 XL's density 3.5 so the look is
// unchanged on this device while scaling correctly on others (38×3.5≈133, etc.).

/// Status-bar strip height, dp.
pub const STATUS_BAR_DP: u32 = 38;
/// Taskbar (Back/Home/Recents nav) strip height, dp.
pub const TASKBAR_DP: u32 = 43;
/// Soft-keyboard default occlusion before the IME reports its real height, dp.
pub const KEYBOARD_DEFAULT_DP: u32 = 343;

/// Per-display physical facts + the chrome/keyboard/orientation **policy** the
/// arbiter authors. This is the WM module's slice of the [`Store`]; the computed
/// rects it pushes to hosts are derived from these fields.
///
/// `panel_w`/`panel_h`/`density` are reported up by the host (`report-panel`);
/// the chrome heights/insets the arbiter pushes are `dp × density` from the dp
/// constants above (Arbiter Inc. 3b — true-dp). Lengths on the wire are physical
/// px (the host applies them verbatim).
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
    /// Whether the foreground app pins orientation (chrome-coherence): when set,
    /// the arbiter fans `orient=0` (portrait) to chrome/IME overlays regardless of
    /// the device sensor. Replaces the cross-process orient-lock file — the
    /// foreground host reports it via `set-orientation-lock`.
    pub orientation_locked: bool,
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
            orientation_locked: false,
        }
    }
}

impl DisplayGeometry {
    /// True once the host has reported the panel density (`report-panel`); until
    /// then the arbiter can't author px and falls back to host-owned chrome.
    pub fn density_known(&self) -> bool {
        self.density > 0.0
    }

    /// Convert density-independent px to physical px for this display. `0` when
    /// the density isn't known yet (caller should fall back to host-owned).
    pub fn dp_to_px(&self, dp: u32) -> u32 {
        if !self.density_known() {
            return 0;
        }
        (dp as f32 * self.density).round() as u32
    }

    /// The authored chrome insets `(status_bar_px, taskbar_px)` = the chrome
    /// strip heights every surface gets on the geometry wire (fullscreen reserves
    /// them; chrome sizes its strip to them; the IME anchors off them).
    pub fn chrome_insets(&self) -> (u32, u32) {
        (self.dp_to_px(STATUS_BAR_DP), self.dp_to_px(TASKBAR_DP))
    }
}

/// A sensor's static descriptor (from the HAL enumerate) plus its last raw
/// reading — the arbiter's per-sensor slice of the [`Store`] (task 77). The
/// binary's sensor driver seeds `max_range`/`resolution` at enumerate so the
/// pure sensors module can derive the proximity near/far threshold from the
/// hardware (no hardcoded distance — [[feedback_no_hardcoding]]); the module
/// updates `last_*` on each [`Event::SensorReading`] so the `sensor-state` verb
/// and a freshly-attached consumer read current state without a fresh sample.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SensorSlot {
    /// HAL `maxRange` — the saturation value (proximity: the "far" distance).
    pub max_range: f32,
    /// HAL `resolution` — the smallest reportable step (the debounce dead-band).
    pub resolution: f32,
    pub last_x: f32,
    pub last_y: f32,
    pub last_z: f32,
    pub last_ts_ns: u64,
    /// False until the first reading lands (distinguishes a real 0.0 from unset).
    pub has_reading: bool,
}

/// The single source of truth. Holds the per-display [`DisplayState`] (geometry
/// policy + the surface/role stack + resource-focus) AND the arbiter-global app
/// registry + home designation + IME intrinsic height (task 74 C — moved off the
/// binary's `state.rs` singletons so the responsibility modules own them via
/// [`Ctx`]). The registry isn't display-scoped; surfaces reference its pids.
#[derive(Debug)]
pub struct Store {
    displays: HashMap<DisplayId, DisplayState>,
    /// Running-apps registry, keyed by app-id (see [`registry`]).
    pub(crate) apps: HashMap<String, AppState>,
    /// The designated home/launcher app-id (task 57), or `None`.
    pub(crate) home: Option<String>,
    /// The soft keyboard's reported intrinsic height, px (task 68).
    pub(crate) ime_height: u32,
    /// Scheduled timed-wake alarms (Arbiter Inc. 3c — see [`alarm`]).
    pub(crate) alarms: Vec<Alarm>,
    /// Active user notifications (Signal bg-receipt M3 — see [`notify`]).
    /// In-memory (transient); not persisted with the registry.
    pub(crate) notifications: Vec<Notification>,
    /// Monotonic global notification handle counter (the surfacer/click key).
    pub(crate) next_nid: u64,
    /// Per-sensor descriptor + last reading (task 77 — SensorService). Seeded by
    /// the binary's sensor driver at enumerate; updated on each reading. Runtime
    /// only (re-seeded on every boot; not persisted with the registry).
    pub(crate) sensors: HashMap<SensorKind, SensorSlot>,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            displays: HashMap::new(),
            apps: HashMap::new(),
            home: None,
            ime_height: DEFAULT_IME_HEIGHT_PX,
            alarms: Vec::new(),
            notifications: Vec::new(),
            next_nid: 0,
            sensors: HashMap::new(),
        }
    }
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a display's full state (geometry + surfaces + focus), if touched.
    pub fn display(&self, id: DisplayId) -> Option<&DisplayState> {
        self.displays.get(&id)
    }

    /// Mutable access, inserting a [`DisplayState::default`] the first time a
    /// display is referenced (`get_or_default`).
    pub fn display_mut(&mut self, id: DisplayId) -> &mut DisplayState {
        self.displays.entry(id).or_default()
    }

    /// Read a display's geometry policy, if touched.
    pub fn geometry(&self, id: DisplayId) -> Option<&DisplayGeometry> {
        self.displays.get(&id).map(|d| &d.geometry)
    }

    /// Mutable access to a display's geometry policy (`get_or_default`).
    pub fn geometry_mut(&mut self, id: DisplayId) -> &mut DisplayGeometry {
        &mut self.displays.entry(id).or_default().geometry
    }

    /// Every display id currently in the store.
    pub fn display_ids(&self) -> Vec<DisplayId> {
        self.displays.keys().copied().collect()
    }

    /// Read a sensor's slot (descriptor + last reading), if known.
    pub fn sensor(&self, kind: SensorKind) -> Option<&SensorSlot> {
        self.sensors.get(&kind)
    }

    /// Mutable access to a sensor's slot (`get_or_default`). Used by the sensors
    /// module to cache readings and by the binary's driver to seed descriptors.
    pub fn sensor_mut(&mut self, kind: SensorKind) -> &mut SensorSlot {
        self.sensors.entry(kind).or_default()
    }

    /// Seed a sensor's static descriptor (HAL enumerate → driver). Preserves any
    /// existing last reading.
    pub fn set_sensor_descriptor(&mut self, kind: SensorKind, max_range: f32, resolution: f32) {
        let slot = self.sensors.entry(kind).or_default();
        slot.max_range = max_range;
        slot.resolution = resolution;
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
    /// An editor's keyboard focus changed. `editor` is always the affected pid
    /// (so a blur push can target the editor that lost focus even after the
    /// resource-focus is cleared in the Store — the focus-follows-foreground
    /// blur targets the *backgrounded* editor, not the visible app). `focused`
    /// is true on gain, false on loss.
    EditorFocusChanged { editor: i32, focused: bool },
    /// A module recomputed a display's geometry (post-recompute notice).
    GeometryRecomputed { id: DisplayId },
    /// A tracked surface's process exited. Emitted by the binary's death
    /// watcher so a module can prune the surface + re-reconcile (task 74).
    SurfaceRemoved { pid: i32 },
    /// The binary's alarm timer ticked (unix epoch ms). The alarm module fires
    /// any due alarms (Arbiter Inc. 3c). Emitted only while alarms exist.
    AlarmTick { now_ms: u64 },
    /// The binary's screen poller reported the display power state (`live` =
    /// On/Vr; false = Off/Doze). The power module (`wandr-arbiter-power`) applies
    /// the doze grace + decides dozing, fanning the cadence to hosts. PowerManager.
    ScreenState { live: bool },

    /// wandr-arbiter-audio (M3b) — a comms session (call) started/ended on `pid`.
    /// The power module keeps the call host OUT of doze while active (no dozing
    /// mid-call). Emitted by the audio module on call-start / call-end.
    CommsActive { pid: i32, active: bool },

    // ── SensorService (task 77) ─────────────────────────────────────────────
    /// A consumer wants `kind` enabled (the **consumer protocol** — modules
    /// never call the sensors module directly, they emit intent). The sensors
    /// module ref-counts `requester` per kind and enables the HAL on the first
    /// holder. `requester` is the holding pid (or a synthetic id) so a later
    /// `SurfaceRemoved`/release drops exactly one reference.
    SensorAcquire { kind: SensorKind, requester: i32 },
    /// A consumer no longer needs `kind`. The sensors module drops `requester`'s
    /// reference and disables the HAL when the last holder releases.
    SensorRelease { kind: SensorKind, requester: i32 },
    /// A raw HAL sample for `kind` (the binary's sensor driver thread bus-emits
    /// these). The sensors module caches it in the Store and translates it to a
    /// semantic event (e.g. [`Event::ProximityChanged`]).
    SensorReading { kind: SensorKind, x: f32, y: f32, z: f32, ts_ns: u64 },
    /// Semantic: the proximity sensor crossed the (descriptor-derived, debounced)
    /// near/far threshold. Consumers react (e.g. screen-off during a call —
    /// follow-on). `near` = an object is close to the panel.
    ProximityChanged { near: bool },
    /// The binary's inactivity ticker fired (PowerManager screen-off-timeout role,
    /// task 86 follow-on). Emitted periodically under `--no-art` (where there is no
    /// PowerManagerService); the power module checks input-idle elapsed vs the
    /// screen-off timeout and sleeps the panel when exceeded. Monotonic, no payload.
    IdleTick,
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

/// How a process is launched (mirrors the binary's launch kinds). Carried on
/// [`Effect::Launch`] so a module can request a launch without touching the
/// zygote itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchKind {
    Gui,
    GuiOverlay,
    Headless,
}

impl LaunchKind {
    /// Stable wire/persistence token (alarm `wake_kind`, schedule-alarm verb).
    pub fn as_wire(&self) -> &'static str {
        match self {
            LaunchKind::Gui => "gui",
            LaunchKind::GuiOverlay => "gui-overlay",
            LaunchKind::Headless => "headless",
        }
    }
    /// Parse a wire token; unknown → `Headless` (the safe default for a wake).
    pub fn from_wire(s: &str) -> Self {
        match s {
            "gui" => LaunchKind::Gui,
            "gui-overlay" => LaunchKind::GuiOverlay,
            _ => LaunchKind::Headless,
        }
    }
}

/// A hardware sensor kind the arbiter's SensorService (task 77) arbitrates.
/// The wire tokens mirror the `skiko-gfx` `sensors` WIT `kind` set so the
/// `report-sensor` sim verb and any future cross-process plumbing share one
/// vocabulary. Only the kinds a consumer needs are enumerated; extend `+1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SensorKind {
    Proximity,
    Accelerometer,
    Light,
    /// HAL-fused screen-rotation sensor (`android.sensor.device_orientation`,
    /// type 27): on-change, reports the rotation index 0/1/2/3 in the reading's
    /// `x`. The arbiter's sensor-driver enables it always-on and the WM turns its
    /// readings into auto-rotation (the native source the old `wandr-sensors`
    /// daemon + its accel math replaced under `--no-art`; task 94).
    DeviceOrientation,
}

impl SensorKind {
    /// Stable wire/log token (the `report-sensor <kind>` arg).
    pub fn as_wire(&self) -> &'static str {
        match self {
            SensorKind::Proximity => "proximity",
            SensorKind::Accelerometer => "accelerometer",
            SensorKind::Light => "light",
            SensorKind::DeviceOrientation => "device-orientation",
        }
    }
    /// Parse a wire token; `None` for an unknown kind (the verb rejects it).
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "proximity" => Some(SensorKind::Proximity),
            "accelerometer" | "accel" => Some(SensorKind::Accelerometer),
            "light" => Some(SensorKind::Light),
            "device-orientation" | "device_orientation" | "orientation" => {
                Some(SensorKind::DeviceOrientation)
            }
            _ => None,
        }
    }
}

/// A side-effect a module *requests*; the binary is the only place that
/// actually performs it (raw signals, `/proc` writes, zygote IPC, sockets,
/// threads). This generalizes the task-73 "`deliver_to_host` queues a push the
/// binary flushes" pattern to every mechanism the AMS/IMMS responsibilities
/// drive. Modules stay pure — they emit `Effect`s; the binary's
/// `execute_effects` runs them in emission order.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Apply a role to a surface. The binary maps [`Role`] to the matching
    /// signal, OOM-score, and present push (one effect replaces the scattered
    /// `send_role_signal`/`write_oom_score`/`send_present` trios).
    SetRole { pid: i32, role: Role },
    /// Ask the zygote to launch an app (home-fallback / launch verb).
    Launch { app_id: String, kind: LaunchKind },
    /// Bring a tracked app to the foreground (the binary runs the `foreground`
    /// verb). No-op if the app isn't tracked (dead) — pair with `Launch` first
    /// for an open-from-dead (e.g. a notification tap). Signal bg-receipt M3.
    Foreground { app_id: String },
    /// Ask the zygote to kill a tracked pid.
    Kill { pid: i32 },
    /// Persist the durable slice (registry / home / foreground) to disk.
    Persist,
    /// A one-line push to a host's control socket (`wandr-host-<pid>`). `line`
    /// must already end with `\n`. (Was the only kind of effect in task 73.)
    HostLine { pid: i32, line: String },
    /// Turn the primary display panel on/off via SurfaceFlinger `setPowerMode`
    /// (task 78). Requested by `wandr-arbiter-power` when proximity says "near"
    /// during a call (off) and on uncover / call-end (on). The binary performs it
    /// via `wandr-hal-display`. Single primary display for now (generalize to a
    /// `DisplayId` when multi-display lands).
    SetDisplayPower { on: bool },
    /// Enable or disable a hardware sensor on the HAL (task 77). The binary's
    /// sensor driver thread is the only place this is performed; the pure
    /// sensors module emits it when the per-`kind` ref-count goes 0→1 (`on:true`)
    /// or 1→0 (`on:false`). `rate_hz` is the requested sample rate (ignored on
    /// disable / on-change sensors). This is the battery contract — a sensor
    /// only draws power while a consumer holds it.
    SetSensor { kind: SensorKind, on: bool, rate_hz: u32 },
    /// Set the primary display's backlight to a normalized brightness fraction
    /// (0.0–1.0) — auto-brightness (task 86). Emitted by `wandr-arbiter-power` from
    /// the ambient-light curve (and a manual override). A *fraction* (not raw
    /// units) keeps the pure module device-independent; the binary maps it to the
    /// panel's raw range (sysfs `max_brightness`, or SurfaceFlinger
    /// `setDisplayBrightness` where the HWC supports it). Distinct from
    /// `SetDisplayPower` (panel on/off): brightness only matters while the panel
    /// is on, so the power module gates it on `panel_on && !blanked`.
    /// `sensor` tags the source: `true` = auto-brightness (the ambient-light curve)
    /// → the Lights HAL `BrightnessMode::SENSOR` (the vendor HAL may smooth it);
    /// `false` = a manual override / on-off default → `BrightnessMode::USER`.
    SetBacklight { level: f32, sensor: bool },
}

/// The handle a module uses during a command or event reaction. It exposes
/// the [`Store`] and two outboxes — emitted [`Event`]s (fanned to every module
/// to a fixpoint) and requested [`Effect`]s (run by the binary after the
/// handler returns). A fresh `Ctx` is created per `on_command` / `on_event`
/// call; its outboxes are drained by the [`Registry`].
pub struct Ctx<'a> {
    pub store: &'a mut Store,
    events: Vec<Event>,
    effects: Vec<Effect>,
}

impl<'a> Ctx<'a> {
    fn new(store: &'a mut Store) -> Self {
        Self { store, events: Vec::new(), effects: Vec::new() }
    }

    /// Emit an event for other modules to react to. Cascaded to a fixpoint by
    /// the [`Registry`] after the current handler returns.
    pub fn emit(&mut self, e: Event) {
        self.events.push(e);
    }

    /// Request a side-effect the binary will perform (in emission order).
    pub fn request(&mut self, eff: Effect) {
        self.effects.push(eff);
    }

    /// Queue a one-line push to a host's control socket — sugar for
    /// [`Effect::HostLine`]. `line` should already end with `\n`.
    pub fn deliver_to_host(&mut self, pid: i32, line: impl Into<String>) {
        self.request(Effect::HostLine { pid, line: line.into() });
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
                    "wandr-arbiter-core: verb {v:?} registered by two modules (idx {prev} and {idx})"
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
    /// it triggered. Returns the reply + every [`Effect`] accumulated across
    /// the command and the cascade. `None` if no module owns `verb` (the
    /// binary then falls through to its legacy match).
    pub fn dispatch_command(
        &mut self,
        verb: &str,
        args: &str,
        store: &mut Store,
    ) -> Option<(Reply, Vec<Effect>)> {
        let idx = *self.verb_index.get(verb)?;
        let (reply, events, effects) = {
            let mut ctx = Ctx::new(store);
            let reply = self.modules[idx].on_command(verb, args, &mut ctx);
            (reply, ctx.events, ctx.effects)
        };
        let all_effects = self.drain_cascade(store, events, effects);
        Some((reply, all_effects))
    }

    /// Inject an event from outside the module system (a legacy handler that
    /// changed shared state the modules care about) and run the cascade.
    /// Returns every [`Effect`] the reacting modules produced.
    pub fn dispatch_event(&mut self, ev: Event, store: &mut Store) -> Vec<Effect> {
        self.drain_cascade(store, vec![ev], Vec::new())
    }

    /// Fan a batch of events to every module's `on_event`, looping over any
    /// events those reactions emit until none remain (fixpoint), accumulating
    /// all effects in emission order. Bounded by [`MAX_CASCADE_DEPTH`].
    fn drain_cascade(
        &mut self,
        store: &mut Store,
        mut events: Vec<Event>,
        mut effects: Vec<Effect>,
    ) -> Vec<Effect> {
        let mut depth = 0usize;
        while !events.is_empty() {
            depth += 1;
            if depth > MAX_CASCADE_DEPTH {
                log::warn!(
                    "wandr-arbiter-core: event cascade exceeded depth {MAX_CASCADE_DEPTH}; \
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
                    effects.extend(ctx.effects);
                }
            }
        }
        effects
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
        let (reply, effects) = reg
            .dispatch_command("echo", "hi", &mut store)
            .expect("module owns echo");
        assert_eq!(reply.render(), "OK echoed hi");
        // The command push, plus the cascade reaction push — both as HostLine.
        assert_eq!(
            effects,
            vec![
                Effect::HostLine { pid: 42, line: "echo hi\n".to_string() },
                Effect::HostLine { pid: 7, line: "reacted\n".to_string() },
            ]
        );
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
    fn dp_to_px_scales_with_density() {
        let mut g = DisplayGeometry::default();
        assert!(!g.density_known());
        assert_eq!(g.dp_to_px(STATUS_BAR_DP), 0, "unknown density → 0 (host-owned)");
        // Pixel 2 XL: 560 dpi → density 3.5. Back-derived dp ≈ the tuned px
        // (38×3.5=133, 43×3.5=150.5→151, 343×3.5=1200.5→1201 — within 1px).
        g.density = 560.0 / 160.0;
        assert_eq!(g.chrome_insets(), (133, 151));
        assert_eq!(g.dp_to_px(KEYBOARD_DEFAULT_DP), 1201);
        // Resolution-independence: a 320-dpi panel (density 2.0) → smaller px.
        g.density = 2.0;
        assert_eq!(g.chrome_insets(), (76, 86));
    }

    #[test]
    fn geometry_store_is_get_or_default() {
        let mut store = Store::new();
        assert!(store.geometry(PRIMARY_DISPLAY).is_none());
        store.geometry_mut(PRIMARY_DISPLAY).inset_top = 96;
        assert_eq!(store.geometry(PRIMARY_DISPLAY).unwrap().inset_top, 96);
    }
}
