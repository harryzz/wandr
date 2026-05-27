//! Running-apps registry — task 46 step 3.
//!
//! Tracks which apps the arbiter has asked the zygote to launch.
//! Updated by the LAUNCH / KILL socket-command handlers. Currently a
//! flat map keyed by app-id; step 4 will grow per-app metadata
//! (foreground/background state, recency, focus-holder, OOM score).
//!
//! At MVP we keep N=1 entry per app-id — a second LAUNCH of the same
//! app replaces the prior entry (KILLing the prior one first is the
//! caller's choice; the arbiter doesn't enforce that yet).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};

/// Which app-id (if any) is currently foreground. The arbiter
/// guarantees at most one. `None` = nothing in foreground (cold
/// arbiter, or last launch was demoted without a successor).
fn foreground() -> &'static Mutex<Option<String>> {
    static FG: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    FG.get_or_init(|| Mutex::new(None))
}

// ─── IME state (task 47 step 1) ───────────────────────────────────────
//
// Two singletons:
//   - active_ime: the IME app-id currently designated to receive
//     editor-attached events. At most one. Set by `set-ime <app-id>`.
//   - editor_focus: which foreground-app pid has a focused editor
//     right now + its `EditorInfo`. Cleared on detach-editor.
//
// The arbiter routes calls from focused-app → IME (attach/detach) and
// IME → focused-app (commit-text / send-key-event / composing) using
// these two pointers. Cross-process delivery itself (per-host control
// socket) lands in step 2; step 1 just maintains the state.

/// IME app currently registered as the active receiver.
#[derive(Clone, Debug)]
pub struct ActiveIme {
    pub app_id: String,
    pub pid: i32,
}

fn active_ime() -> &'static Mutex<Option<ActiveIme>> {
    static IME: OnceLock<Mutex<Option<ActiveIme>>> = OnceLock::new();
    IME.get_or_init(|| Mutex::new(None))
}

pub fn current_active_ime() -> Option<ActiveIme> {
    active_ime().lock().ok().and_then(|m| m.clone())
}

/// Set / unset the active IME. The caller must guarantee `app_id` (if
/// `Some`) is currently in `registry()` — typically by calling
/// `state::get(app_id)` first to look up the pid.
pub fn set_active_ime(new: Option<ActiveIme>) -> Option<ActiveIme> {
    let mut m = active_ime().lock().ok()?;
    let prev = m.clone();
    *m = new;
    prev
}

/// Editor-info subset of the WIT `war:ime/editor-info` record. Mirrors
/// the on-wire shape exactly. Stored alongside the focused-pid so
/// future `on-editor-attached` re-dispatches (e.g. after an IME swap)
/// can re-send the same metadata without re-querying the focused app.
#[derive(Clone, Debug)]
pub struct EditorInfo {
    pub input_type: String, // one of: text, number, phone, email, url, password, multiline-text
    pub hint: String,
    pub initial_text: String,
    pub initial_selection_start: u32,
    pub initial_selection_end: u32,
}

impl EditorInfo {
    pub fn text_default() -> Self {
        Self {
            input_type: "text".to_string(),
            hint: String::new(),
            initial_text: String::new(),
            initial_selection_start: 0,
            initial_selection_end: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EditorFocus {
    pub pid: i32,
    pub editor_info: EditorInfo,
}

fn editor_focus() -> &'static Mutex<Option<EditorFocus>> {
    static EF: OnceLock<Mutex<Option<EditorFocus>>> = OnceLock::new();
    EF.get_or_init(|| Mutex::new(None))
}

pub fn current_editor_focus() -> Option<EditorFocus> {
    editor_focus().lock().ok().and_then(|m| m.clone())
}

/// Set the focused editor. Returns the prior focus (if any), so the
/// caller can dispatch `on-editor-detached` to the previous focused
/// app's IME before sending `on-editor-attached` to the new one.
pub fn set_editor_focus(new: Option<EditorFocus>) -> Option<EditorFocus> {
    let mut m = editor_focus().lock().ok()?;
    let prev = m.clone();
    *m = new;
    prev
}

// ─── Overlay state (task 47 step 3c) ──────────────────────────────────
//
// Tracks the IME-overlay split: which IME pid is the foreground overlay,
// and which app pid is the editor "behind" it (visible-but-demoted,
// not Paused). Used by `cmd_overlay`/`cmd_overlay_clear` and the
// auto-tie inside `cmd_attach_editor`/`cmd_detach_editor`.
//
// Distinct from `foreground()` — when overlay is active, fg points at
// the IME (so SIGUSR2 routes to the IME), and `behind_pid` records the
// editor process that received SIGRTMIN+1 (OverlayBehind). On overlay
// clear, `behind_pid` is repromoted back to foreground.

#[derive(Clone, Debug)]
pub struct OverlayState {
    /// The IME process that owns the bottom-strip overlay surface.
    pub ime_pid: i32,
    /// The app whose surface is behind the overlay (visible, layer 0,
    /// lifecycle Resumed). Repromoted on overlay clear.
    pub behind_pid: i32,
}

fn overlay_state() -> &'static Mutex<Option<OverlayState>> {
    static OV: OnceLock<Mutex<Option<OverlayState>>> = OnceLock::new();
    OV.get_or_init(|| Mutex::new(None))
}

pub fn current_overlay() -> Option<OverlayState> {
    overlay_state().lock().ok().and_then(|m| m.clone())
}

pub fn set_overlay(new: Option<OverlayState>) -> Option<OverlayState> {
    let mut m = overlay_state().lock().ok()?;
    let prev = m.clone();
    *m = new;
    prev
}

pub fn current_foreground() -> Option<String> {
    foreground().lock().ok().and_then(|m| m.clone())
}

/// Swap the foreground app. Returns the previously-foreground
/// `(app_id, pid)` pair (if any) so the caller can SIGUSR1 it.
pub fn set_foreground(new_app_id: Option<&str>) -> Option<(String, i32)> {
    let prev_app_id = {
        let mut fg = foreground().lock().ok()?;
        let prev = fg.clone();
        *fg = new_app_id.map(|s| s.to_string());
        prev
    };
    let prev = prev_app_id?;
    if Some(prev.as_str()) == new_app_id {
        return None;
    }
    let pid = get(&prev)?.pid;
    Some((prev, pid))
}

/// One running app instance.
#[derive(Clone, Debug)]
pub struct AppState {
    pub app_id: String,
    pub pid: i32,
    /// When the LAUNCH socket-command landed. `Instant` for elapsed
    /// math + `SystemTime` for human-readable timestamps.
    pub launched_at: SystemTime,
    pub launched_mono: Instant,
}

/// Process-wide registry. `OnceLock<Mutex<…>>` so we can hand out a
/// `&'static Mutex` without forcing every caller to thread a handle.
fn registry() -> &'static Mutex<HashMap<String, AppState>> {
    static REG: OnceLock<Mutex<HashMap<String, AppState>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn insert(state: AppState) {
    if let Ok(mut m) = registry().lock() {
        m.insert(state.app_id.clone(), state);
    }
}

pub fn remove(app_id: &str) -> Option<AppState> {
    // If the removed app was foreground, also clear the fg slot —
    // arbiter callers will repromote another if there's a successor.
    if current_foreground().as_deref() == Some(app_id) {
        let _ = set_foreground(None);
    }
    // Task 47 step 1 — if the removed app was the active IME, clear
    // that pointer too. The arbiter caller will fall back to "no IME"
    // (or re-promote a different installed IME).
    if current_active_ime().map(|i| i.app_id) == Some(app_id.to_string()) {
        let _ = set_active_ime(None);
    }
    // If the removed app had a focused editor, clear it — the editor
    // is dead with its host process.
    let removed = registry().lock().ok().and_then(|mut m| m.remove(app_id));
    if let Some(ref s) = removed {
        if current_editor_focus().map(|f| f.pid) == Some(s.pid) {
            let _ = set_editor_focus(None);
        }
        // Task 47 step 3c — if the removed app was either side of the
        // overlay split, tear the split down. The surviving side stays
        // running; the arbiter caller can repromote it to fg if it
        // was the behind-app.
        if let Some(ov) = current_overlay() {
            if ov.ime_pid == s.pid || ov.behind_pid == s.pid {
                let _ = set_overlay(None);
            }
        }
    }
    removed
}

pub fn get(app_id: &str) -> Option<AppState> {
    registry().lock().ok().and_then(|m| m.get(app_id).cloned())
}

/// Snapshot for the LIST command. Cheap because the map is small at
/// MVP scales (~tens of apps max).
pub fn snapshot() -> Vec<AppState> {
    registry().lock()
        .map(|m| m.values().cloned().collect())
        .unwrap_or_default()
}

// ─── Persistence (task 46 crash-marker) ────────────────────────────────
//
// The arbiter is the parent of the wart-arbiter.sock listener and the
// owner of the (app_id → pid) mapping — but the children themselves
// are forked by the zygote, not by the arbiter. If the arbiter
// crashes or restarts, the children survive, but the arbiter's
// in-memory map is gone. Without persistence, `wart-arbiter list`
// returns empty after restart even though apps are still running.
//
// Save shape (hand-rolled JSON — keeping the arbiter dep-free):
//
//   {
//     "version": 1,
//     "saved_at_unix": 1730000000,
//     "foreground": "com.example.wart-app",   // or null
//     "apps": [
//       {"app_id":"com.example.wart-app","pid":4293,"launched_at_unix":1730000000},
//       ...
//     ]
//   }
//
// On restore_from(): kill(pid, 0) each app's pid — alive → re-insert,
// dead → drop + log. Same prune for the foreground field (cleared if
// no longer in the live set).

/// Write the current state to `path` atomically (`<path>.tmp` → rename).
pub fn save_to(path: &Path) -> Result<()> {
    let apps = snapshot();
    let fg = current_foreground();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Hand-rolled JSON to avoid a serde dep on a binary that's
    // deliberately tiny. Strings are app-ids (alphanumeric + dot +
    // hyphen + underscore in practice) so naive escaping suffices —
    // but defensively escape backslash + dquote anyway.
    let mut json = String::new();
    json.push_str("{\n");
    json.push_str("  \"version\": 1,\n");
    json.push_str(&format!("  \"saved_at_unix\": {now},\n"));
    match &fg {
        Some(f) => json.push_str(&format!("  \"foreground\": \"{}\",\n", json_escape(f))),
        None    => json.push_str("  \"foreground\": null,\n"),
    }
    json.push_str("  \"apps\": [");
    for (i, app) in apps.iter().enumerate() {
        if i > 0 { json.push_str(", "); }
        let launched = app.launched_at
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        json.push_str(&format!(
            "\n    {{\"app_id\": \"{}\", \"pid\": {}, \"launched_at_unix\": {}}}",
            json_escape(&app.app_id), app.pid, launched,
        ));
    }
    if !apps.is_empty() { json.push_str("\n  "); }
    json.push_str("]\n");
    json.push_str("}\n");

    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)
        .with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} → {}", tmp.display(), path.display()))?;
    Ok(())
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Restore from `path`. Verifies each persisted pid is still alive via
/// `kill(pid, 0)` — dead pids are dropped + logged; live pids are
/// re-inserted with the original `app_id` + `pid` + `launched_at`.
/// Foreground is cleared if the persisted fg pid is dead.
///
/// Returns `(restored_alive, dropped_dead)`. A missing file is not
/// an error — returns `Ok((0, 0))` so cold daemon starts go through
/// the same call site.
pub fn restore_from(path: &Path) -> Result<(usize, usize)> {
    if !path.exists() {
        return Ok((0, 0));
    }
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;

    // Tiny ad-hoc JSON walker — sufficient for our fixed schema. If
    // the schema grows we should pull in serde-json then.
    let entries = parse_apps(&body)?;
    let fg = parse_foreground(&body);

    let mut alive = 0usize;
    let mut dead = 0usize;
    for (app_id, pid, launched_secs) in entries {
        if pid_alive(pid) {
            let launched_at = UNIX_EPOCH + std::time::Duration::from_secs(launched_secs);
            insert(AppState {
                app_id,
                pid,
                launched_at,
                launched_mono: Instant::now(), // restart-relative is fine for LIST display
            });
            alive += 1;
        } else {
            log::info!(
                "arbiter: state restore — pid {} for app {:?} is dead, dropping",
                pid, app_id,
            );
            dead += 1;
        }
    }

    if let Some(fg_id) = fg {
        if get(&fg_id).is_some() {
            // The fg app survived; re-install the fg pointer.
            let _ = set_foreground(Some(&fg_id));
        } else {
            log::info!("arbiter: state restore — foreground app {fg_id:?} not alive, fg cleared");
        }
    }

    Ok((alive, dead))
}

/// kill(pid, 0) probes for liveness without delivering a signal.
/// 0 → alive (and we have permission); -1 + ESRCH → dead.
fn pid_alive(pid: i32) -> bool {
    if pid <= 0 { return false; }
    let r = unsafe { libc::kill(pid, 0) };
    if r == 0 { return true; }
    let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    // EPERM = alive but we lack permission to signal — still alive.
    err == libc::EPERM
}

fn parse_apps(body: &str) -> Result<Vec<(String, i32, u64)>> {
    // Find `"apps": [ ... ]` then split entries by `},`.
    let Some(open) = body.find("\"apps\"") else { return Ok(vec![]); };
    let Some(arr_start) = body[open..].find('[') else { return Ok(vec![]); };
    let arr_start = open + arr_start + 1;
    let Some(arr_end_rel) = body[arr_start..].find(']') else {
        return Err(anyhow!("malformed state.json — apps[] unterminated"));
    };
    let arr = &body[arr_start..arr_start + arr_end_rel];

    let mut out = Vec::new();
    for chunk in arr.split('}') {
        let Some(open) = chunk.find('{') else { continue };
        let entry = &chunk[open + 1..];
        let app_id = pick_str(entry, "app_id").unwrap_or_default();
        let pid    = pick_int(entry, "pid").unwrap_or(0) as i32;
        let launched = pick_int(entry, "launched_at_unix").unwrap_or(0) as u64;
        if !app_id.is_empty() && pid > 0 {
            out.push((app_id, pid, launched));
        }
    }
    Ok(out)
}

fn parse_foreground(body: &str) -> Option<String> {
    // Look for `"foreground": "...",` (string) or `"foreground": null,`
    let idx = body.find("\"foreground\"")?;
    let after = &body[idx..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    if rest.starts_with("null") {
        return None;
    }
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn pick_str(entry: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let idx = entry.find(&needle)?;
    let after = &entry[idx + needle.len()..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn pick_int(entry: &str, key: &str) -> Option<i64> {
    let needle = format!("\"{key}\"");
    let idx = entry.find(&needle)?;
    let after = &entry[idx + needle.len()..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    let end = rest.find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}
