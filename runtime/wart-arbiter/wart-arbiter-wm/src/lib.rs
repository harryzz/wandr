//! wart-arbiter-wm — the WindowManager module (task 73).
//!
//! The first real responsibility crate, absorbing the WMS geometry policy
//! that was duplicated in every per-app host: chrome insets, soft-keyboard
//! occlusion, and orientation. Android's WMS *decided* layout/insets and
//! handed off to SurfaceFlinger; it never rendered. Same here — this module
//! decides and pushes a single `geometry` line per surface; the host applies
//! it and runs its own skia matrix (render mechanism).
//!
//! It plugs into [`wart_arbiter_core`] as an [`ArbiterModule`]: it owns the
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
//! - **M2**: the arbiter is given the chrome heights (WART_INSET_* in its own
//!   env) ⇒ it authors real insets; the host stops sourcing them at runtime.
//! - **M3**: the host starts reporting orientation up ⇒ the arbiter decides
//!   it and pushes a real `orient`; the host applies it on push-back.

use wart_arbiter_core::{
    ArbiterModule, Ctx, DisplayId, Event, Reply, Role, Store, INSET_HOST_OWNED, ORIENT_HOST_OWNED,
    PRIMARY_DISPLAY,
};

/// Map the Device Orientation HAL value (`Surface.ROTATION_*` index) to the
/// renderer's dihedral content-rotation code. Ported verbatim from the host's
/// `device_rotation_to_orient` (wart-host/src/standalone.rs) — moving this
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
                let orient = device_rotation_to_orient(raw);
                ctx.emit(Event::OrientationChanged { id: PRIMARY_DISPLAY, orient });
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
                log::info!("wart-arbiter-wm: report-panel w={w} h={h} dpi={dpi} density={density:.3} → insets ({top},{bottom})");
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

            // Not geometry-relevant to the WM.
            Event::DisplayAdded { .. }
            | Event::PanelMeasured { .. }
            | Event::InsetsChanged { .. }
            | Event::GeometryRecomputed { .. }
            | Event::SurfaceRemoved { .. }
            | Event::AlarmTick { .. }
            | Event::ScreenState { .. }
            | Event::CommsActive { .. }
            | Event::SensorAcquire { .. }
            | Event::SensorRelease { .. }
            | Event::SensorReading { .. }
            | Event::ProximityChanged { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wart_arbiter_core::{EditorInfo, Effect, Registry, Role};

    /// Shorthand for the `HostLine` effect the WM module emits.
    fn hl(pid: i32, line: &str) -> Effect {
        Effect::HostLine { pid, line: line.to_string() }
    }

    fn reg_with_wm() -> (Registry, Store) {
        // Force M1 (no env insets) for deterministic tests regardless of the
        // ambient environment.
        std::env::remove_var("WART_INSET_TOP");
        std::env::remove_var("WART_INSET_BOTTOM");
        let mut reg = Registry::new();
        reg.register(Box::new(WmModule::new()));
        (reg, Store::new())
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
        store.display_mut(PRIMARY_DISPLAY).put_surface(70, "war.statusbar", Role::Chrome);
        store.display_mut(PRIMARY_DISPLAY).put_surface(80, "war.ime.keyboard", Role::Background);
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
        store.display_mut(PRIMARY_DISPLAY).put_surface(70, "war.statusbar", Role::Chrome);
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
        store.display_mut(PRIMARY_DISPLAY).put_surface(70, "war.statusbar", Role::Chrome);
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
