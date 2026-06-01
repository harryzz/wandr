//! The per-display surface/role model + generalized resource-focus (task 74).
//!
//! This is the load-bearing core schema the design doc
//! (`docs/visual-sizing-design-patterns.md`) says design effort belongs in: a
//! typed, per-display set of **surfaces** with **roles**, plus a generalized
//! **resource-focus** map. It replaces the arbiter's ad-hoc singletons
//! (`foreground`/`overlay_state`/`editor_focus`/`active_ime`) and dissolves the
//! "foreground slot points at the IME during an overlay split" knot:
//!
//! - The IME is a [`Surface`] with [`Role::Overlay`]; the app behind it is a
//!   [`Surface`] with [`Role::OverlayBehind`] — two honest surfaces, no slot
//!   that lies.
//! - "What is the user looking at" is a **derivation** ([`DisplayState::visible_app`]),
//!   not a stored value plus an `active_app_pid()` corrector.
//! - "Should the IME overlay be shown" is a **pure function**
//!   ([`DisplayState::overlay_desired`]) over one [`DisplayState`].
//!
//! Focus is modeled as a *resource* (the doc's "resource-focus arbiter"): a map
//! from [`ResourceKind`] to its holder, so audio-focus / input-focus /
//! notifications are each "+1 variant", never a refactor.

use std::collections::HashMap;

use crate::DisplayGeometry;

/// What a surface is *for* right now. Replaces the `foreground: Option<String>`
/// slot + the `OverlayState { overlay_pid, behind_pid }` pair. Each maps to a
/// host role-mechanism the *binary* applies (the module only sets the role):
///
/// | Role           | host mechanism                       |
/// |----------------|--------------------------------------|
/// | `Foreground`   | SIGUSR2 + OOM_FG + present           |
/// | `Overlay`      | SIGUSR2 + OOM_FG + present           |
/// | `OverlayBehind`| SIGRTMIN+1 + OOM_FG                   |
/// | `Background`   | SIGUSR1 + OOM_BG                      |
/// | `Chrome`       | (anchored; declared now, engaged later) |
/// | `Headless`     | none (no SF surface)                 |
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The app the user is looking at; top of stack when no overlay is engaged.
    Foreground,
    /// Visible app demoted *below* an engaged overlay (IME): layer 0, still
    /// Resumed + rendering. (Was `OverlayState.behind_pid`.)
    OverlayBehind,
    /// An overlay-drawing guest promoted above an `OverlayBehind` app — the IME
    /// today, a side/utility panel later. (Was "the IME in the foreground slot".)
    Overlay,
    /// Not visible, lifecycle Paused/Stopped.
    Background,
    /// Anchored chrome (statusbar/taskbar). Declared for the deferred
    /// chrome-coherence work; not engaged this increment.
    Chrome,
    /// Launched headless (no SF surface); role signals are N/A.
    Headless,
    /// The keyguard/lockscreen — a full-screen surface above the app + nav (but
    /// below the status bar), shown + focused while the device is locked. Mapped
    /// to the foreground *mechanism* (show + focus) by the binary, but excluded
    /// from `visible_app`/the task-cycle ring (it is not a switchable app). The
    /// app it covers is demoted to `Background` so it stops fighting for focus.
    Lockscreen,
}

/// One tracked GUI process on a display. The `pid` is the stable identity (it
/// keys every existing signal/socket path); `app_id` is carried for AM-level
/// reasoning + `list`.
#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub pid: i32,
    pub app_id: String,
    pub role: Role,
}

/// Editor metadata — moved verbatim from the binary's `state.rs::EditorInfo`,
/// because the focused editor is now core resource-focus state. Mirrors the
/// `war:ime/editor-info` WIT record so the IME re-dispatch path is unchanged.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct EditorInfo {
    /// One of: text, number, phone, email, url, password, multiline-text.
    pub input_type: String,
    pub hint: String,
    pub initial_text: String,
    pub initial_selection_start: u32,
    pub initial_selection_end: u32,
}

/// A focusable resource. Generalized now so audio-focus / input-focus /
/// notifications later are `+1` variants, not a refactor (the doc's bake-in-now
/// decision). At most one holder per kind per display.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    /// The focused text editor (drives IME overlay visibility).
    ImeEditor,
    // Deferred, additive: Window, Input, Audio, Notification.
}

/// The holder of a [`ResourceKind`] + any per-resource payload.
#[derive(Clone, Debug, PartialEq)]
pub enum ResourceFocus {
    /// `pid` owns the focused editor; `info` is its metadata. (Was
    /// `editor_focus: Option<EditorFocus>`.)
    ImeEditor { pid: i32, info: EditorInfo },
    // Deferred, additive: Window(pid), Input(pid), Audio(pid), Notification {..}.
}

/// Everything the WMS/AMS/IMMS responsibilities share for one display: the
/// geometry policy (existing), the surface stack, the resource-focus map, and
/// the active-IME designation.
#[derive(Debug, Default)]
pub struct DisplayState {
    /// Geometry policy (chrome insets, keyboard occlusion, orientation) — the
    /// task-73 slice, now nested here.
    pub geometry: DisplayGeometry,
    /// The surfaces on this display, keyed by pid (at most one Foreground / one
    /// OverlayBehind / one Overlay at a time, by construction).
    surfaces: Vec<Surface>,
    /// Generalized resource-focus.
    focus: HashMap<ResourceKind, ResourceFocus>,
    /// Which surface (pid) is the active IME — the surface *allowed* to be the
    /// `Overlay` when an editor focuses. (Was `active_ime: Option<ActiveIme>`.)
    pub active_ime: Option<i32>,
}

impl DisplayState {
    // ── surface stack ────────────────────────────────────────────────────

    /// Insert or update a surface (by pid). Idempotent on role.
    pub fn put_surface(&mut self, pid: i32, app_id: impl Into<String>, role: Role) {
        let app_id = app_id.into();
        if let Some(s) = self.surfaces.iter_mut().find(|s| s.pid == pid) {
            s.app_id = app_id;
            s.role = role;
        } else {
            self.surfaces.push(Surface { pid, app_id, role });
        }
    }

    /// Set a surface's role (no-op if the pid isn't present).
    pub fn set_role(&mut self, pid: i32, role: Role) {
        if let Some(s) = self.surfaces.iter_mut().find(|s| s.pid == pid) {
            s.role = role;
        }
    }

    /// Remove a surface and any resource-focus / active-IME it held.
    pub fn remove_surface(&mut self, pid: i32) {
        self.surfaces.retain(|s| s.pid != pid);
        if self.active_ime == Some(pid) {
            self.active_ime = None;
        }
        if self.ime_editor().map(|(p, _)| p) == Some(pid) {
            self.focus.remove(&ResourceKind::ImeEditor);
        }
    }

    /// Drop every surface + focus (used by the Step-A wholesale mirror rebuild).
    pub fn clear(&mut self) {
        self.surfaces.clear();
        self.focus.clear();
        self.active_ime = None;
    }

    pub fn surfaces(&self) -> &[Surface] {
        &self.surfaces
    }

    pub fn role_of(&self, pid: i32) -> Option<Role> {
        self.surfaces.iter().find(|s| s.pid == pid).map(|s| s.role)
    }

    // ── resource-focus ───────────────────────────────────────────────────

    /// The focused editor `(pid, &info)`, if any.
    pub fn ime_editor(&self) -> Option<(i32, &EditorInfo)> {
        match self.focus.get(&ResourceKind::ImeEditor)? {
            ResourceFocus::ImeEditor { pid, info } => Some((*pid, info)),
        }
    }

    /// Set / clear the focused editor.
    pub fn set_ime_editor(&mut self, v: Option<(i32, EditorInfo)>) {
        match v {
            Some((pid, info)) => {
                self.focus
                    .insert(ResourceKind::ImeEditor, ResourceFocus::ImeEditor { pid, info });
            }
            None => {
                self.focus.remove(&ResourceKind::ImeEditor);
            }
        }
    }

    // ── derivations (pure; no signals, no I/O) ───────────────────────────

    /// The pid of the app the user is actually looking at. The knot dissolves:
    /// it's the `OverlayBehind` surface when a split is engaged, else the
    /// `Foreground` surface. No "foreground slot points at the IME" branch —
    /// the IME is `Role::Overlay`, never counted here. Exact parity with the
    /// legacy `active_app_pid()`.
    pub fn visible_app(&self) -> Option<i32> {
        self.surfaces
            .iter()
            .find(|s| s.role == Role::OverlayBehind)
            .or_else(|| self.surfaces.iter().find(|s| s.role == Role::Foreground))
            .map(|s| s.pid)
    }

    /// Whether the IME overlay should be shown, and over whom — returns the IME
    /// pid when desired. Desired iff: an active IME exists, an editor is
    /// focused, that editor belongs to the visible app, and the IME isn't that
    /// same app. (Was `reconcile_overlay`'s `desired` computation.)
    pub fn overlay_desired(&self) -> Option<i32> {
        let ime = self.active_ime?;
        let (editor, _) = self.ime_editor()?;
        let vis = self.visible_app()?;
        (editor == vis && ime != vis).then_some(ime)
    }

    // ── legacy-read shims (task 74 Step D) ───────────────────────────────
    //
    // The arbiter's command handlers used to read four singletons
    // (`foreground`/`active_ime`/`editor_focus`/`overlay_state`). These methods
    // re-express those exact reads over the model so the singletons can be
    // deleted with no behavior change.

    /// The surface a given `pid` owns (carries `app_id`), if tracked.
    pub fn surface(&self, pid: i32) -> Option<&Surface> {
        self.surfaces.iter().find(|s| s.pid == pid)
    }

    /// The first surface in a given role. At most one `Foreground` /
    /// `OverlayBehind` / `Overlay` exists by construction, so "first" == "the".
    pub fn first_with_role(&self, role: Role) -> Option<&Surface> {
        self.surfaces.iter().find(|s| s.role == role)
    }

    /// The legacy `foreground` *slot* equivalent: the `Overlay` surface when a
    /// split is engaged, else the `Foreground` surface. During a split this is
    /// the **IME** — the slot that "lied" — which is intentional: `cmd_back` /
    /// `cmd_list` / persistence preserve their exact legacy semantics by reading
    /// this. Distinct from [`Self::visible_app`] (which is the app *behind* the
    /// overlay); use that for "what the user is looking at".
    pub fn foreground_slot(&self) -> Option<&Surface> {
        self.first_with_role(Role::Overlay)
            .or_else(|| self.first_with_role(Role::Foreground))
    }

    /// Whether an overlay split is currently engaged (an `Overlay` surface
    /// exists). (Was `state::current_overlay().is_some()`.)
    pub fn overlay_engaged(&self) -> bool {
        self.first_with_role(Role::Overlay).is_some()
    }

    /// The active-IME surface (carries `app_id` for the host-route + display
    /// text). (Was `state::current_active_ime()`.)
    pub fn active_ime_surface(&self) -> Option<&Surface> {
        self.surface(self.active_ime?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ds_with(roles: &[(i32, Role)]) -> DisplayState {
        let mut d = DisplayState::default();
        for (pid, role) in roles {
            d.put_surface(*pid, format!("app{pid}"), *role);
        }
        d
    }

    #[test]
    fn visible_app_prefers_overlay_behind() {
        // No overlay: the foreground app is visible.
        let d = ds_with(&[(10, Role::Foreground), (11, Role::Background)]);
        assert_eq!(d.visible_app(), Some(10));
        // Overlay engaged: the IME is Role::Overlay, the app is OverlayBehind,
        // and the *visible* app is the behind app — never the IME.
        let d = ds_with(&[(20, Role::Overlay), (10, Role::OverlayBehind)]);
        assert_eq!(d.visible_app(), Some(10));
    }

    #[test]
    fn overlay_desired_mirrors_reconcile() {
        let mut d = ds_with(&[(10, Role::Foreground)]);
        d.active_ime = Some(20);
        d.put_surface(20, "ime", Role::Background);
        // No editor focus yet → not desired.
        assert_eq!(d.overlay_desired(), None);
        // Editor focus on the visible app, IME is a different pid → desired.
        d.set_ime_editor(Some((10, EditorInfo::default())));
        assert_eq!(d.overlay_desired(), Some(20));
        // Editor focus on some other (backgrounded) pid → not desired.
        d.set_ime_editor(Some((11, EditorInfo::default())));
        assert_eq!(d.overlay_desired(), None);
    }

    #[test]
    fn foreground_slot_is_overlay_then_foreground() {
        // No overlay: the slot is the foreground app (== visible_app).
        let d = ds_with(&[(10, Role::Foreground), (11, Role::Background)]);
        assert_eq!(d.foreground_slot().map(|s| s.pid), Some(10));
        assert_eq!(d.visible_app(), Some(10));
        assert!(!d.overlay_engaged());
        // Overlay engaged: the slot is the IME (the "lie"); visible is the app.
        let d = ds_with(&[(20, Role::Overlay), (10, Role::OverlayBehind)]);
        assert_eq!(d.foreground_slot().map(|s| s.pid), Some(20));
        assert_eq!(d.visible_app(), Some(10));
        assert!(d.overlay_engaged());
    }

    #[test]
    fn active_ime_surface_resolves_app_id() {
        let mut d = ds_with(&[(10, Role::Foreground)]);
        assert!(d.active_ime_surface().is_none());
        d.put_surface(20, "ime", Role::Background);
        d.active_ime = Some(20);
        assert_eq!(d.active_ime_surface().map(|s| s.app_id.as_str()), Some("ime"));
    }

    #[test]
    fn remove_surface_clears_focus_and_ime() {
        let mut d = ds_with(&[(10, Role::Foreground), (20, Role::Background)]);
        d.active_ime = Some(20);
        d.set_ime_editor(Some((10, EditorInfo::default())));
        d.remove_surface(20);
        assert_eq!(d.active_ime, None);
        d.remove_surface(10);
        assert!(d.ime_editor().is_none());
    }
}
