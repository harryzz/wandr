//! wandr-arbiter-wm — the WindowManager module (task 73).
//!
//! The first real responsibility crate, absorbing the WMS geometry policy
//! that was duplicated in every per-app host: chrome insets, soft-keyboard
//! occlusion, and orientation. Android's WMS *decided* layout/insets and
//! handed off to SurfaceFlinger; it never rendered. Same here — this module
//! decides and pushes a single `geometry` line per surface; the host applies
//! it and runs its own skia matrix (render mechanism).
//!
//! It plugs into [`wandr_arbiter_core`] as an [`ArbiterModule`]: it owns the
//! `report-orientation` verb (the host→arbiter sensor report-up) and reacts
//! to the geometry-relevant events the binary's legacy handlers bridge onto
//! the bus (`ImeHeightChanged`, `EditorFocusChanged`, `ForegroundChanged`).
//! All pushing happens in [`WmModule::on_event`]; commands only translate a
//! report into a domain event.
//!
//! ## The wire it pushes (arbiter → host)
//! ```text
//! geometry <inset_top> <inset_bottom> <keyboard_px> <orient>
//! ```
//! - `inset_top`/`inset_bottom`: chrome reserved at the user top/bottom, px —
//!   or [`INSET_HOST_OWNED`] while the arbiter hasn't been told the chrome
//!   heights (host keeps its env-sourced insets).
//! - `keyboard_px`: how much the soft keyboard occludes **this** surface
//!   (the keyboard's height for the focused editor, `0` for any other).
//! - `orient`: the decided dihedral code `0..=7`, or [`ORIENT_HOST_OWNED`]
//!   while the host remains the rotation authority for this surface.
//!
//! ## Staging (self-gating — no code change between milestones)
//! - **M1** (byte-equivalent): no chrome-height config and no orientation
//!   report yet ⇒ inset fields are [`INSET_HOST_OWNED`], `orient` is
//!   [`ORIENT_HOST_OWNED`]. Only the keyboard occlusion actually moves; the
//!   host keeps its env insets + local rotation. Verifies the kernel + wire.
//! - **M2**: the arbiter is given the chrome heights (WANDR_INSET_* in its own
//!   env) ⇒ it authors real insets; the host stops sourcing them at runtime.
//! - **M3**: the host starts reporting orientation up ⇒ the arbiter decides
//!   it and pushes a real `orient`; the host applies it on push-back.

use wandr_arbiter_core::{
    ArbiterModule, ChromeAnchor, Ctx, DisplayId, Event, Reply, Role, SensorKind, Store,
    INSET_HOST_OWNED, ORIENT_HOST_OWNED, PRIMARY_DISPLAY,
};

/// Map the Device Orientation HAL value (`Surface.ROTATION_*` index) to the
/// renderer's dihedral content-rotation code. Ported verbatim from the host's
/// `device_rotation_to_orient` (wandr-host/src/standalone.rs) — moving this
/// device→content mapping here is the WMS-policy half of the move; the host
/// keeps its copy only for the no-arbiter fallback.
///
/// The HAL reports the rotation the *display* should adopt; we pre-rotate the
/// *content* by the inverse: 0→0 (identity), 1→4 (ROT_90), 2→3 (ROT_180),
/// 3→7 (ROT_270). The `1⇒4 / 3⇒7` handedness is the one panel-specific
/// assumption; it must stay in lockstep with the host's copy.
fn device_rotation_to_orient(rot: u32) -> u32 {
    match rot & 3 {
        0 => 0, // portrait — identity
        1 => 4, // ROT_90
        2 => 3, // ROT_180
        _ => 7, // ROT_270
    }
}

/// WindowManager module — owns per-display geometry policy. The authoritative
/// geometry AND the targeting state (focused editor, visible app) both live in
/// the core [`Store`] now (task 74 E): the module reads `ime_editor()` /
/// `visible_app()` for the no-payload re-push handlers (ImeHeight / Orientation)
/// and uses the pid carried on the focus/foreground events otherwise — no
/// private targeting caches that could drift from the model.
pub struct WmModule;

/// The focused editor's pid for a display, read from the Store's resource-focus
/// (the single source) — used by the no-payload re-push handlers.
fn focused_editor(store: &Store, id: DisplayId) -> Option<i32> {
    store.display(id).and_then(|d| d.ime_editor().map(|(p, _)| p))
}

/// The **system orientation** every visible surface displays at: `0` (portrait)
/// when the foreground app pins orientation, else the decided device orient
/// (`ORIENT_HOST_OWNED` until the arbiter has decided one). The lock is global —
/// gating ONLY chrome (not the foreground app) is what made a locked launcher
/// rotate while the bars stayed portrait. So this gates *everyone*.
fn effective_orient(store: &Store, id: DisplayId) -> u32 {
    store
        .geometry(id)
        .map(|g| if g.orientation_locked { 0 } else { g.orientation })
        .unwrap_or(ORIENT_HOST_OWNED)
}

impl Default for WmModule {
    fn default() -> Self {
        Self::new()
    }
}

impl WmModule {
    pub fn new() -> Self {
        WmModule
    }

    /// The inset fields to put on the wire: the arbiter-authored chrome heights
    /// (`dp × density`) once the host has reported panel density, else the
    /// host-owned sentinel (pre-report). These ARE the chrome strip heights —
    /// every surface gets them (fullscreen reserves; chrome sizes; IME anchors).
    fn inset_fields(store: &Store, id: DisplayId) -> (u32, u32) {
        match store.geometry(id) {
            Some(g) if g.density_known() => g.chrome_insets(),
            _ => (INSET_HOST_OWNED, INSET_HOST_OWNED),
        }
    }

    /// True once the arbiter has any policy worth pushing to a non-editor
    /// surface: authored insets (density known) or a decided orientation. Gates
    /// the foreground push so a plain app switch with no policy stays a no-op.
    fn has_surface_policy(&self, store: &Store, id: DisplayId) -> bool {
        store
            .geometry(id)
            .map(|g| g.density_known() || g.orientation != ORIENT_HOST_OWNED)
            .unwrap_or(false)
    }

    /// Build the `geometry` line for `id`, with `keyboard_px` set to this
    /// surface's keyboard occlusion (caller decides: the keyboard height for
    /// the focused editor, `0` otherwise).
    fn geometry_line(&self, store: &Store, id: DisplayId, keyboard_px: u32) -> String {
        let orient = effective_orient(store, id);
        let (top, bottom) = Self::inset_fields(store, id);
        format!("geometry {top} {bottom} {keyboard_px} {orient}\n")
    }

    /// The keyboard's current intrinsic height for `id` (px; 0 if unknown).
    fn keyboard_px(store: &Store, id: DisplayId) -> u32 {
        store.geometry(id).map(|g| g.keyboard_px).unwrap_or(0)
    }

    /// Fan the effective overlay orientation to the chrome + active-IME surfaces
    /// (chrome-coherence). Effective orient = `0` (portrait) when the foreground
    /// app pins orientation, else the decided orient. The target set is bounded:
    /// every `Role::Chrome` surface + the `active_ime` pid (so the IME stays
    /// orient-fresh even while hidden → no stale-on-engage). Chrome/IME ignore
    /// insets (`INSET_HOST_OWNED`) and carry no keyboard inset on this push (the
    /// IME's own height path is separate). Each overlay applies its own anchor
    /// flip via `overlay_rect`, so only the content `orient` is sent.
    fn fan_overlays(&self, ctx: &mut Ctx, id: DisplayId) {
        // Nothing decided yet (host-owned) → no orient to fan.
        if effective_orient(ctx.store, id) == ORIENT_HOST_OWNED {
            return;
        }
        let mut targets: Vec<i32> = ctx
            .store
            .display(id)
            .map(|d| {
                d.surfaces()
                    .iter()
                    .filter(|s| s.role == Role::Chrome)
                    .map(|s| s.pid)
                    .collect()
            })
            .unwrap_or_default();
        if let Some(ime) = ctx.store.display(id).and_then(|d| d.active_ime) {
            if !targets.contains(&ime) {
                targets.push(ime);
            }
        }
        let line = self.geometry_line(ctx.store, id, 0);
        for pid in targets {
            ctx.deliver_to_host(pid, line.clone());
        }
    }

    /// Translate a raw device rotation (`Surface.ROTATION_*` index 0..3) to the
    /// content dihedral and emit [`Event::OrientationChanged`] for the primary
    /// display. Shared by the `report-orientation` verb (desktop/sim + the host
    /// fallback) and the live device-orientation sensor reading (task 94). Returns
    /// the content orient for the verb's reply.
    fn apply_device_rotation(&self, raw: u32, ctx: &mut Ctx) -> u32 {
        let orient = device_rotation_to_orient(raw);
        ctx.emit(Event::OrientationChanged { id: PRIMARY_DISPLAY, orient });
        orient
    }

    /// Push the current **system orientation** ([`effective_orient`]) to every
    /// surface that should display it: the focused editor (with its keyboard
    /// inset), the visible app, and the chrome + active-IME overlays. Shared by
    /// `OrientationChanged` (the device rotated) and `set-orientation-lock` (the
    /// foreground lock toggled). The lock gates ALL of them uniformly, so a locked
    /// foreground app and the chrome stay coherent (both portrait).
    fn push_system_orientation(&self, ctx: &mut Ctx, id: DisplayId) {
        let editor = focused_editor(ctx.store, id);
        if let Some(ed) = editor {
            let kb = Self::keyboard_px(ctx.store, id);
            let line = self.geometry_line(ctx.store, id, kb);
            ctx.deliver_to_host(ed, line);
        }
        if let Some(fg) = ctx.store.display(id).and_then(|d| d.visible_app()) {
            if Some(fg) != editor {
                let line = self.geometry_line(ctx.store, id, 0);
                ctx.deliver_to_host(fg, line);
            }
        }
        self.fan_overlays(ctx, id);
    }
}

// ── Input-window authoring (task 84) ─────────────────────────────────────────
//
// Under ART-off there is no system_server WindowManager, so SurfaceFlinger never
// pushes WindowInfos to the standalone InputDispatcher (wandr-inputflinger) — every
// touch is dropped ("no touchable window"). The arbiter *is* the WMS, so it authors
// the ordered input-window list itself and pushes it to wandr-inputflinger, which
// feeds the dispatcher via `onWindowInfosChanged`. This is the same decide-don't-
// render split as the `geometry` push: the arbiter owns roles + insets + orient +
// focus; the rects are derived from them (never hardcoded).

/// Input z-order policy — the single source of truth for how surface roles stack
/// for *hit-testing* (distinct from the render/OOM role mechanics). Higher value =
/// closer to the user = hit-tested first. Chrome splits by anchor: the status bar
/// sits above the lockscreen; the taskbar/nav sits below it (the keyguard covers
/// nav but not the status bar — see [`Role::Lockscreen`]).
mod input_z {
    pub const STATUS_BAR: i32 = 100;
    pub const LOCKSCREEN: i32 = 90;
    pub const IME: i32 = 85;
    pub const TASKBAR: i32 = 80;
    pub const APP: i32 = 10;
}

/// The display-space rect a chrome/IME strip occupies for a given content
/// orientation — a faithful mirror of the host's `overlay_rect`
/// (wandr-host/src/standalone.rs) so the strip's INPUT region tracks where it
/// actually RENDERS after the device rotates (in landscape the bars move to the
/// physical sides). Without this the input rects stay portrait top/bottom strips
/// while the bars draw on the sides → taps miss. `at_bottom`/`th`/`off` mirror
/// `overlay_rect`'s per-strip tuple; the panel buffer is fixed portrait `pw×ph`.
/// Returns `(l, t, r, b)`. Handedness (0→S/3→N/4→W/7→E, else portrait) MUST stay
/// in lockstep with the host's `user_bottom_edge`.
fn strip_rect(at_bottom: bool, th: i32, off: i32, orient: u32, pw: i32, ph: i32) -> (i32, i32, i32, i32) {
    #[derive(Clone, Copy)]
    enum Edge {
        N,
        S,
        W,
        E,
    }
    let user_bottom = match orient {
        3 => Edge::N, // 180°
        4 => Edge::W, // 90°  — physical left
        7 => Edge::E, // 270° — physical right
        _ => Edge::S, // portrait (0) / host-owned (255)
    };
    let edge = if at_bottom {
        user_bottom
    } else {
        match user_bottom {
            Edge::S => Edge::N,
            Edge::N => Edge::S,
            Edge::W => Edge::E,
            Edge::E => Edge::W,
        }
    };
    // A `th`-thick, full-span strip `off` px inward from `edge` (overlay_rect's
    // (x,y,w,h), converted to (l,t,r,b)).
    match edge {
        Edge::S => (0, ph - off - th, pw, ph - off),
        Edge::N => (0, off, pw, off + th),
        Edge::W => (off, 0, off + th, ph),
        Edge::E => (pw - off - th, 0, pw - off, ph),
    }
}

/// One authored input window: its display-space rect, z, focusability, and pid.
struct InputWin {
    pid: i32,
    z: i32,
    l: i32,
    t: i32,
    r: i32,
    b: i32,
    /// Whether this window can hold key focus. Chrome + IME are touch-only
    /// (keys go to the editor/app); app + lockscreen are focusable.
    focusable: bool,
}

/// Build the ordered input-window block the arbiter pushes to wandr-inputflinger,
/// or `None` if there's nothing to author yet (panel dims unknown — pre
/// report-panel — or no visible surface). Pure derivation over the [`Store`]:
///
/// ```text
/// win-begin <orient> <panel_w> <panel_h>
/// win <pid> <l> <t> <r> <b> <focusable:0|1>   (repeated, TOP→BOTTOM z-order)
/// win-focus <pid|-1>
/// win-commit
/// ```
///
/// Rects come from the live insets + keyboard occlusion + panel dims, so they
/// track rotation/keyboard changes with no hardcoded geometry. The app surface is
/// authored fullscreen (the host renders fullscreen); chrome/IME sit above it by
/// z-order, so their strips win where they overlap and taps elsewhere fall through
/// to the app — exactly the stock-Android stacking.
pub fn input_window_block(store: &Store, id: DisplayId) -> Option<String> {
    let disp = store.display(id)?;
    let g = store.geometry(id)?;
    if g.panel_w == 0 || g.panel_h == 0 {
        return None; // no panel measured yet → can't place anything
    }
    let (w, h) = (g.panel_w as i32, g.panel_h as i32);
    let orient = effective_orient(store, id);
    // Chrome strip heights from the authored insets (0 when not yet known → no
    // strip authored, app still routes).
    let (inset_top, inset_bottom) = match g.chrome_insets() {
        (t, b) if t != INSET_HOST_OWNED && b != INSET_HOST_OWNED => (t as i32, b as i32),
        _ => (0, 0),
    };
    let kb = g.keyboard_px as i32;

    let mut wins: Vec<InputWin> = Vec::new();
    for s in disp.surfaces() {
        let win = match s.role {
            // Fullscreen, focusable. The app the user is looking at.
            Role::Foreground | Role::OverlayBehind => {
                InputWin { pid: s.pid, z: input_z::APP, l: 0, t: 0, r: w, b: h, focusable: true }
            }
            // Fullscreen lockscreen above the app + nav, below the status bar.
            Role::Lockscreen => InputWin {
                pid: s.pid,
                z: input_z::LOCKSCREEN,
                l: 0,
                t: 0,
                r: w,
                b: h,
                focusable: true,
            },
            // IME overlay — keyboard strip, touch-only. `strip_rect` places it like
            // the host's `overlay_rect`: at the user's bottom edge, `inset_bottom`
            // inward (just above the taskbar), thickness = keyboard depth. In
            // landscape this is a side strip, matching where the keyboard renders.
            // Skip if no height.
            Role::Overlay => {
                if kb <= 0 {
                    continue;
                }
                let (l, t, r, b) = strip_rect(true, kb, inset_bottom, orient, w, h);
                InputWin { pid: s.pid, z: input_z::IME, l, t, r, b, focusable: false }
            }
            // Chrome strip placed from its anchor via `strip_rect` (rotation-aware,
            // mirrors the host's `overlay_rect`), touch-only.
            Role::Chrome => match s.anchor {
                Some(ChromeAnchor::Top) => {
                    if inset_top <= 0 {
                        continue;
                    }
                    // status bar — the user's TOP edge (at_bottom = false).
                    let (l, t, r, b) = strip_rect(false, inset_top, 0, orient, w, h);
                    InputWin { pid: s.pid, z: input_z::STATUS_BAR, l, t, r, b, focusable: false }
                }
                Some(ChromeAnchor::Bottom) => {
                    if inset_bottom <= 0 {
                        continue;
                    }
                    // taskbar — the user's BOTTOM edge (at_bottom = true).
                    let (l, t, r, b) = strip_rect(true, inset_bottom, 0, orient, w, h);
                    InputWin { pid: s.pid, z: input_z::TASKBAR, l, t, r, b, focusable: false }
                }
                None => continue, // unplaced chrome — don't guess
            },
            // Not visible / no SF surface → no input window.
            Role::Background | Role::Headless => continue,
        };
        wins.push(win);
    }
    if wins.is_empty() {
        return None;
    }
    // Top→bottom: highest z first (the dispatcher returns the first touchable
    // window containing the point). Stable so equal-z order is deterministic.
    wins.sort_by(|a, b| b.z.cmp(&a.z));

    // Key focus: the lockscreen when locked, else the visible app. Chrome/IME are
    // never focus targets (touch-only).
    let focus_pid = disp
        .first_with_role(Role::Lockscreen)
        .map(|s| s.pid)
        .or_else(|| disp.visible_app())
        .unwrap_or(-1);

    let mut out = String::new();
    out.push_str(&format!("win-begin {orient} {w} {h}\n"));
    for win in &wins {
        out.push_str(&format!(
            "win {} {} {} {} {} {}\n",
            win.pid, win.l, win.t, win.r, win.b, win.focusable as u8
        ));
    }
    out.push_str(&format!("win-focus {focus_pid}\n"));
    out.push_str("win-commit\n");
    Some(out)
}

impl ArbiterModule for WmModule {
    fn verbs(&self) -> &[&'static str] {
        &["report-orientation", "set-orientation-lock", "report-panel"]
    }

    fn on_command(&mut self, verb: &str, args: &str, ctx: &mut Ctx) -> Reply {
        match verb {
            // `report-orientation <raw 0..3>` — the foreground host's sensor
            // reports a screen rotation. Translate it (device→content dihedral)
            // and emit `OrientationChanged`; the event handler applies it + fans
            // to the surfaces we reach (fullscreen applies on push-back, with a
            // local fallback if we're unreachable).
            "report-orientation" => {
                let Some(raw_tok) = args.split_whitespace().next() else {
                    return Reply::err("report-orientation-missing-raw");
                };
                let Ok(raw) = raw_tok.parse::<u32>() else {
                    return Reply::err(format!("report-orientation-bad-raw {raw_tok:?}"));
                };
                let orient = self.apply_device_rotation(raw, ctx);
                Reply::ok(format!("orientation raw={raw} orient={orient}"))
            }
            // `set-orientation-lock <0|1>` — the foreground fullscreen app reports
            // whether it pins orientation (chrome-coherence; replaces the
            // orient-lock file). Record it + re-fan the effective orient to the
            // chrome/IME overlays so they snap on a foreground/lock change.
            "set-orientation-lock" => {
                let locked = match args.trim() {
                    "1" => true,
                    "0" => false,
                    other => return Reply::err(format!("set-orientation-lock-bad-arg {other:?}")),
                };
                ctx.store.geometry_mut(PRIMARY_DISPLAY).orientation_locked = locked;
                // Re-push the system orientation to everyone: the foreground app
                // (un)rotates and the chrome snaps with it (coherent).
                self.push_system_orientation(ctx, PRIMARY_DISPLAY);
                Reply::ok(format!("orientation-lock {}", locked as u8))
            }
            // `report-panel <w> <h> <dpi>` — the host reports the panel size +
            // density (true-dp). The arbiter stores it (density = dpi/160) and now
            // authors the chrome heights/insets as dp×density. The REPLY carries
            // the authored insets so the caller can `set_insets` before its first
            // frame (no launch-time push/socket race). On the first/changed
            // density, re-push geometry to existing surfaces (chrome that
            // registered before density was known, the visible app, the IME).
            "report-panel" => {
                let toks: Vec<&str> = args.split_whitespace().collect();
                if toks.len() != 3 {
                    return Reply::err("report-panel-args: expected <w> <h> <dpi>");
                }
                let (Ok(w), Ok(h), Ok(dpi)) =
                    (toks[0].parse::<u32>(), toks[1].parse::<u32>(), toks[2].parse::<u32>())
                else {
                    return Reply::err(format!("report-panel-bad-args {args:?}"));
                };
                let density = dpi as f32 / 160.0;
                let changed = {
                    let g = ctx.store.geometry_mut(PRIMARY_DISPLAY);
                    let changed = (g.density - density).abs() > f32::EPSILON
                        || g.panel_w != w
                        || g.panel_h != h;
                    g.panel_w = w;
                    g.panel_h = h;
                    g.density = density;
                    changed
                };
                let (top, bottom) = ctx
                    .store
                    .geometry(PRIMARY_DISPLAY)
                    .map(|g| g.chrome_insets())
                    .unwrap_or((INSET_HOST_OWNED, INSET_HOST_OWNED));
                if changed {
                    ctx.emit(Event::PanelMeasured { id: PRIMARY_DISPLAY, w, h, density });
                    // Density just became known/changed → refresh everyone's geometry.
                    self.push_system_orientation(ctx, PRIMARY_DISPLAY);
                }
                log::info!("wandr-arbiter-wm: report-panel w={w} h={h} dpi={dpi} density={density:.3} → insets ({top},{bottom})");
                Reply::ok(format!(
                    "panel w={w} h={h} density={density:.3} inset-top={top} inset-bottom={bottom}"
                ))
            }
            other => Reply::err(format!("wm-unknown-verb {other}")),
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut Ctx) {
        match ev {
            // The soft keyboard's intrinsic height changed (task 68 seam). Record
            // it; if an editor is focused (read from the Store — no payload), re-
            // push so its bottom inset tracks a live keyboard resize. This is also
            // the push that engages the inset on attach (the shell sets the editor
            // in the Store, then co-emits this event — see EditorFocusChanged).
            Event::ImeHeightChanged { id, px } => {
                ctx.store.geometry_mut(*id).keyboard_px = *px;
                if let Some(ed) = focused_editor(ctx.store, *id) {
                    let line = self.geometry_line(ctx.store, *id, *px);
                    ctx.deliver_to_host(ed, line);
                }
            }

            // Editor keyboard focus changed (carries the affected pid). On gain,
            // the co-emitted `ImeHeightChanged` already pushed the inset to this
            // editor (read from the Store), so this is a no-op — pushing here too
            // would double on attach. On loss, restore the editor's inset
            // (keyboard_px = 0); we target the carried pid, which on a focus-
            // follows-foreground blur is the *backgrounded* editor, not the
            // visible app.
            Event::EditorFocusChanged { editor, focused } => {
                if !*focused {
                    let line = self.geometry_line(ctx.store, PRIMARY_DISPLAY, 0);
                    ctx.deliver_to_host(*editor, line);
                }
            }

            // A new app came forward (carries the pid). Once the arbiter authors
            // any surface policy (insets in M2, orientation in M3) it pushes fresh
            // geometry to the newly-visible app. In M1 there is no such policy, so
            // this is a no-op (keeps the plain app-switch byte-equivalent).
            Event::ForegroundChanged { pid, .. } => {
                if let Some(fg) = *pid {
                    if self.has_surface_policy(ctx.store, PRIMARY_DISPLAY) {
                        // The newly-foregrounded app has no editor focus yet, so
                        // the keyboard doesn't occlude it.
                        let line = self.geometry_line(ctx.store, PRIMARY_DISPLAY, 0);
                        ctx.deliver_to_host(fg, line);
                    }
                }
            }

            // The arbiter decided a new device orient (from report-orientation).
            // Record it, then push the system orientation (gated by the foreground
            // lock) to EVERY surface — focused editor, visible app, chrome +
            // active-IME. Gating the visible app too (not just chrome) keeps a
            // locked foreground app and the chrome coherent: a stray rotation
            // report (e.g. from a backgrounded sensor) can't rotate the locked app
            // while the bars stay portrait.
            Event::OrientationChanged { id, orient } => {
                ctx.store.geometry_mut(*id).orientation = *orient;
                self.push_system_orientation(ctx, *id);
            }

            // Live HAL device-orientation reading (task 94) → auto-rotation. The
            // rotation index is in `x` (0/1/2/3). Act only on a CHANGE: the
            // `OrientationChanged` handler re-pushes geometry to every surface, so
            // re-emitting on an identical reading would thrash. The native source
            // that replaced the old `wandr-sensors` daemon under `--no-art`.
            Event::SensorReading { kind: SensorKind::DeviceOrientation, x, .. } => {
                let raw = (x.round() as i64).rem_euclid(4) as u32;
                let orient = device_rotation_to_orient(raw);
                if ctx.store.geometry(PRIMARY_DISPLAY).map(|g| g.orientation) != Some(orient) {
                    self.apply_device_rotation(raw, ctx);
                }
            }

            // Not geometry-relevant to the WM (other sensor kinds handled by their
            // own modules: proximity → sensors, light → power).
            Event::DisplayAdded { .. }
            | Event::PanelMeasured { .. }
            | Event::InsetsChanged { .. }
            | Event::GeometryRecomputed { .. }
            | Event::SurfaceRemoved { .. }
            | Event::AlarmTick { .. }
            | Event::ScreenState { .. }
            | Event::CommsActive { .. }
            | Event::VideoCallActive { .. }
            | Event::SensorAcquire { .. }
            | Event::SensorRelease { .. }
            | Event::SensorReading { .. }
            | Event::ProximityChanged { .. }
            | Event::IdleTick => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wandr_arbiter_core::{ChromeAnchor, EditorInfo, Effect, Registry, Role};

    /// Shorthand for the `HostLine` effect the WM module emits.
    fn hl(pid: i32, line: &str) -> Effect {
        Effect::HostLine { pid, line: line.to_string() }
    }

    fn reg_with_wm() -> (Registry, Store) {
        // Force M1 (no env insets) for deterministic tests regardless of the
        // ambient environment.
        std::env::remove_var("WANDR_INSET_TOP");
        std::env::remove_var("WANDR_INSET_BOTTOM");
        let mut reg = Registry::new();
        reg.register(Box::new(WmModule::new()));
        (reg, Store::new())
    }

    /// A Pixel-2-XL-shaped store with the panel measured (560 dpi → density 3.5,
    /// chrome insets 133/151) so `input_window_block` can place strips.
    fn measured_store() -> Store {
        let mut store = Store::new();
        let g = store.geometry_mut(PRIMARY_DISPLAY);
        g.panel_w = 1440;
        g.panel_h = 2880;
        g.density = 3.5;
        store
    }

    #[test]
    fn input_window_block_none_before_panel_measured() {
        let mut store = Store::new();
        store.display_mut(PRIMARY_DISPLAY).put_surface(10, "app", Role::Foreground);
        assert_eq!(input_window_block(&store, PRIMARY_DISPLAY), None);
    }

    #[test]
    fn input_window_block_orders_chrome_above_app() {
        let mut store = measured_store();
        let d = store.display_mut(PRIMARY_DISPLAY);
        d.put_surface(10, "app", Role::Foreground);
        d.put_surface(70, "wandr.statusbar", Role::Chrome);
        d.set_chrome_anchor(70, ChromeAnchor::Top);
        d.put_surface(71, "wandr.taskbar", Role::Chrome);
        d.set_chrome_anchor(71, ChromeAnchor::Bottom);
        // A backgrounded app contributes no input window.
        d.put_surface(20, "bg", Role::Background);
        let block = input_window_block(&store, PRIMARY_DISPLAY).unwrap();
        // Top→bottom: statusbar(133-tall top strip), taskbar(151-tall bottom strip),
        // fullscreen app; chrome touch-only, app focusable + focused.
        let expected = "\
win-begin 255 1440 2880\n\
win 70 0 0 1440 133 0\n\
win 71 0 2729 1440 2880 0\n\
win 10 0 0 1440 2880 1\n\
win-focus 10\n\
win-commit\n";
        assert_eq!(block, expected);
    }

    #[test]
    fn input_window_block_rotates_chrome_in_landscape() {
        let mut store = measured_store();
        store.geometry_mut(PRIMARY_DISPLAY).orientation = 4; // 90° → user-bottom West
        let d = store.display_mut(PRIMARY_DISPLAY);
        d.put_surface(10, "app", Role::Foreground);
        d.put_surface(70, "wandr.statusbar", Role::Chrome);
        d.set_chrome_anchor(70, ChromeAnchor::Top);
        d.put_surface(71, "wandr.taskbar", Role::Chrome);
        d.set_chrome_anchor(71, ChromeAnchor::Bottom);
        let block = input_window_block(&store, PRIMARY_DISPLAY).unwrap();
        // orient=4 (user bottom = physical West): the status bar moves to the
        // physical East edge (right vertical strip, thickness inset_top=133) and the
        // taskbar to the West edge (left vertical strip, inset_bottom=151) — matching
        // where the host renders them. The fullscreen app rect is unchanged.
        let expected = "\
win-begin 4 1440 2880\n\
win 70 1307 0 1440 2880 0\n\
win 71 0 0 151 2880 0\n\
win 10 0 0 1440 2880 1\n\
win-focus 10\n\
win-commit\n";
        assert_eq!(block, expected);
    }

    #[test]
    fn input_window_block_lockscreen_is_focused_above_app() {
        let mut store = measured_store();
        let d = store.display_mut(PRIMARY_DISPLAY);
        // Locked: the app is demoted to Background, the keyguard is fullscreen.
        d.put_surface(10, "app", Role::Background);
        d.put_surface(99, "wandr.keyguard", Role::Lockscreen);
        d.put_surface(70, "wandr.statusbar", Role::Chrome);
        d.set_chrome_anchor(70, ChromeAnchor::Top);
        let block = input_window_block(&store, PRIMARY_DISPLAY).unwrap();
        // statusbar above lockscreen; the backgrounded app is excluded; focus = keyguard.
        let expected = "\
win-begin 255 1440 2880\n\
win 70 0 0 1440 133 0\n\
win 99 0 0 1440 2880 1\n\
win-focus 99\n\
win-commit\n";
        assert_eq!(block, expected);
    }

    #[test]
    fn input_window_block_ime_strip_when_keyboard_up() {
        let mut store = measured_store();
        store.geometry_mut(PRIMARY_DISPLAY).keyboard_px = 1000;
        let d = store.display_mut(PRIMARY_DISPLAY);
        d.put_surface(10, "app", Role::OverlayBehind);
        d.put_surface(30, "wandr.ime", Role::Overlay);
        let block = input_window_block(&store, PRIMARY_DISPLAY).unwrap();
        // IME = 1000px strip sitting ABOVE the taskbar inset (151px @ 3.5 density),
        // i.e. [2880-151-1000, 2880-151] = [1729, 2729], touch-only, above the
        // fullscreen app; the visible app (OverlayBehind) holds focus, not the IME.
        let expected = "\
win-begin 255 1440 2880\n\
win 30 0 1729 1440 2729 0\n\
win 10 0 0 1440 2880 1\n\
win-focus 10\n\
win-commit\n";
        assert_eq!(block, expected);
    }

    /// Model an editor focusing: the shell sets the resource-focus in the Store
    /// (the WM's single source for targeting), then co-emits ImeHeightChanged.
    fn focus_editor(store: &mut Store, pid: i32) {
        store.display_mut(PRIMARY_DISPLAY).put_surface(pid, "app", Role::Foreground);
        store
            .display_mut(PRIMARY_DISPLAY)
            .set_ime_editor(Some((pid, EditorInfo::default())));
    }

    #[test]
    fn m1_keyboard_show_then_resize_then_hide() {
        let (mut reg, mut store) = reg_with_wm();
        // No editor focused yet (Store has none) → height change pushes nothing.
        let p = reg.dispatch_event(Event::ImeHeightChanged { id: PRIMARY_DISPLAY, px: 1200 }, &mut store);
        assert!(p.is_empty(), "no editor focused yet → no push");
        // Editor focuses: shell sets the Store, co-emits ImeHeightChanged (push)
        // + EditorFocusChanged{focused:true} (no-op — height event did the push).
        focus_editor(&mut store, 99);
        let p = reg.dispatch_event(Event::ImeHeightChanged { id: PRIMARY_DISPLAY, px: 1200 }, &mut store);
        assert_eq!(p, vec![hl(99, "geometry 65535 65535 1200 255\n")]);
        let p = reg.dispatch_event(Event::EditorFocusChanged { editor: 99, focused: true }, &mut store);
        assert!(p.is_empty(), "focus-gain is a no-op (ImeHeightChanged pushed)");
        // Keyboard resizes while shown → re-push to the focused editor (Store).
        let p = reg.dispatch_event(Event::ImeHeightChanged { id: PRIMARY_DISPLAY, px: 800 }, &mut store);
        assert_eq!(p, vec![hl(99, "geometry 65535 65535 800 255\n")]);
        // Editor blurs → restore its inset (keyboard_px 0), targeting the carried
        // pid. (Shell clears the Store focus; the WM doesn't read it here.)
        store.display_mut(PRIMARY_DISPLAY).set_ime_editor(None);
        let p = reg.dispatch_event(Event::EditorFocusChanged { editor: 99, focused: false }, &mut store);
        assert_eq!(p, vec![hl(99, "geometry 65535 65535 0 255\n")]);
    }

    #[test]
    fn blur_targets_carried_pid_not_visible_app() {
        // Focus-follows-foreground: editor 99 had the keyboard, app 5 is now
        // visible. The blur push must restore 99 (the backgrounded editor), NOT
        // the visible app 5 — the regression the carried pid prevents.
        let (mut reg, mut store) = reg_with_wm();
        store.display_mut(PRIMARY_DISPLAY).put_surface(5, "vis", Role::Foreground);
        // editor 99 no longer focused in the Store (already cleared on demote).
        let p = reg.dispatch_event(Event::EditorFocusChanged { editor: 99, focused: false }, &mut store);
        assert_eq!(p, vec![hl(99, "geometry 65535 65535 0 255\n")]);
    }

    #[test]
    fn report_panel_authors_dp_insets() {
        let (mut reg, mut store) = reg_with_wm();
        store.display_mut(PRIMARY_DISPLAY).put_surface(5, "app", Role::Foreground);
        // Pixel 2 XL: 560 dpi → density 3.5 → insets (133, 151).
        let (reply, pushes) = reg.dispatch_command("report-panel", "1440 2880 560", &mut store).unwrap();
        assert!(reply.render().contains("inset-top=133"), "{}", reply.render());
        assert!(reply.render().contains("inset-bottom=151"), "{}", reply.render());
        assert_eq!(store.geometry(PRIMARY_DISPLAY).unwrap().density, 3.5);
        // Density known → existing surfaces get a real-inset geometry push.
        assert!(
            pushes.iter().any(|e| matches!(e, Effect::HostLine { pid: 5, line } if line.starts_with("geometry 133 151 0 "))),
            "visible app re-pushed with authored insets: {pushes:?}"
        );
    }

    #[test]
    fn m1_foreground_change_is_noop() {
        let (mut reg, mut store) = reg_with_wm();
        let p = reg.dispatch_event(
            Event::ForegroundChanged { app_id: Some("a".into()), pid: Some(5) },
            &mut store,
        );
        assert!(p.is_empty(), "M1: a plain app switch pushes no geometry");
    }

    #[test]
    fn m3_orientation_decides_and_pushes() {
        let (mut reg, mut store) = reg_with_wm();
        // Foreground app + focused editor live in the Store (same pid here).
        focus_editor(&mut store, 5);
        reg.dispatch_event(Event::ImeHeightChanged { id: PRIMARY_DISPLAY, px: 1000 }, &mut store);
        // Device rotates 90° (raw=1 → dihedral 4). report-orientation verb.
        let (reply, pushes) = reg
            .dispatch_command("report-orientation", "1", &mut store)
            .expect("WM owns report-orientation");
        assert_eq!(reply.render(), "OK orientation raw=1 orient=4");
        // Editor (== visible app) gets geometry with orient 4 + its keyboard inset;
        // the visible-app push is suppressed because it IS the editor.
        assert_eq!(pushes, vec![hl(5, "geometry 65535 65535 1000 4\n")]);
        assert_eq!(store.geometry(PRIMARY_DISPLAY).unwrap().orientation, 4);
    }

    #[test]
    fn rotate_fans_orient_to_chrome_and_active_ime() {
        let (mut reg, mut store) = reg_with_wm();
        store.display_mut(PRIMARY_DISPLAY).put_surface(70, "wandr.statusbar", Role::Chrome);
        store.display_mut(PRIMARY_DISPLAY).put_surface(80, "wandr.ime.keyboard", Role::Background);
        store.display_mut(PRIMARY_DISPLAY).active_ime = Some(80);
        // Device rotates 90° → dihedral 4. Chrome + the (hidden) IME both get it.
        let (_reply, pushes) = reg.dispatch_command("report-orientation", "1", &mut store).unwrap();
        assert!(pushes.contains(&hl(70, "geometry 65535 65535 0 4\n")), "chrome fanned: {pushes:?}");
        assert!(pushes.contains(&hl(80, "geometry 65535 65535 0 4\n")), "active_ime fanned: {pushes:?}");
    }

    #[test]
    fn locked_foreground_app_and_chrome_both_stay_portrait() {
        // Regression: the lock must gate the VISIBLE APP too, not just chrome —
        // else a stray rotation report (e.g. from a backgrounded sensor) rotates
        // the locked foreground app while the bars stay portrait (incoherent).
        let (mut reg, mut store) = reg_with_wm();
        store.display_mut(PRIMARY_DISPLAY).put_surface(5, "launcher", Role::Foreground);
        store.display_mut(PRIMARY_DISPLAY).put_surface(70, "wandr.statusbar", Role::Chrome);
        reg.dispatch_command("set-orientation-lock", "1", &mut store); // foreground pins orientation
        let (_r, pushes) = reg.dispatch_command("report-orientation", "1", &mut store).unwrap();
        // Both the foreground app AND chrome get orient 0 (portrait) — coherent.
        assert!(pushes.contains(&hl(5, "geometry 65535 65535 0 0\n")), "fg gated: {pushes:?}");
        assert!(pushes.contains(&hl(70, "geometry 65535 65535 0 0\n")), "chrome gated: {pushes:?}");
        assert!(
            !pushes.iter().any(|e| matches!(e, Effect::HostLine { line, .. } if line.ends_with(" 4\n"))),
            "nothing rotated to landscape: {pushes:?}"
        );
    }

    #[test]
    fn lock_snaps_chrome_portrait_then_unlock_restores() {
        let (mut reg, mut store) = reg_with_wm();
        store.display_mut(PRIMARY_DISPLAY).put_surface(70, "wandr.statusbar", Role::Chrome);
        // Establish a decided rotated orient.
        reg.dispatch_command("report-orientation", "1", &mut store); // decided = 4
        // Foreground app pins orientation → chrome snaps to portrait (0).
        let (reply, pushes) = reg.dispatch_command("set-orientation-lock", "1", &mut store).unwrap();
        assert_eq!(reply.render(), "OK orientation-lock 1");
        assert_eq!(pushes, vec![hl(70, "geometry 65535 65535 0 0\n")]);
        // Unlock → chrome returns to the decided orient (4).
        let (_r, pushes) = reg.dispatch_command("set-orientation-lock", "0", &mut store).unwrap();
        assert_eq!(pushes, vec![hl(70, "geometry 65535 65535 0 4\n")]);
    }

    #[test]
    fn bad_orientation_arg_errors() {
        let (mut reg, mut store) = reg_with_wm();
        let (reply, pushes) = reg
            .dispatch_command("report-orientation", "nope", &mut store)
            .unwrap();
        assert!(matches!(reply, Reply::Err(_)));
        assert!(pushes.is_empty());
    }
}
