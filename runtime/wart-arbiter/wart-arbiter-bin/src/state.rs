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

// ─── IME overlay height (task 68) ─────────────────────────────────────
//
// How many physical px the soft keyboard occludes. The IME guest is the
// source of truth: its `request-overlay-height` host impl reports the value
// here. The arbiter pushes it as a `keyboard-inset` to the focused editor's
// host on attach so the foreground app reserves room. Default = the IME's
// historical fixed height until it reports (lets the inset work pre-M2).

const DEFAULT_IME_HEIGHT_PX: u32 = 1200;

fn ime_height() -> &'static std::sync::atomic::AtomicU32 {
    static H: std::sync::atomic::AtomicU32 =
        std::sync::atomic::AtomicU32::new(DEFAULT_IME_HEIGHT_PX);
    &H
}

pub fn ime_overlay_height() -> u32 {
    ime_height().load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_ime_overlay_height(px: u32) {
    ime_height().store(px, std::sync::atomic::Ordering::Relaxed);
}

// ─── Home app (task 57 launcher) ──────────────────────────────────────
//
// The app-id the arbiter treats as "home": foregrounded at boot, by the
// `go-home` command, and as the fall-back when the foreground app exits
// or is killed (so the screen never goes black). Persisted across
// arbiter restarts. `None` = no home designated (current pre-launcher
// behavior — empty fg).

fn home_app() -> &'static Mutex<Option<String>> {
    static HOME: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    HOME.get_or_init(|| Mutex::new(None))
}

pub fn current_home() -> Option<String> {
    home_app().lock().ok().and_then(|m| m.clone())
}

/// Set / clear the designated home app-id. Returns the previous value.
pub fn set_home(new: Option<&str>) -> Option<String> {
    let mut m = home_app().lock().ok()?;
    let prev = m.clone();
    *m = new.map(|s| s.to_string());
    prev
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
    // Registry removal only. The foreground / active-IME / editor-focus /
    // overlay teardown now lives entirely in the surface model — callers pair
    // this with `model_remove_surface(pid)` (task 74 D).
    registry().lock().ok().and_then(|mut m| m.remove(app_id))
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
pub fn save_to(path: &Path, foreground_app_id: Option<&str>) -> Result<()> {
    let apps = snapshot();
    let fg = foreground_app_id.map(|s| s.to_string());
    let home = current_home();
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
    match &home {
        Some(h) => json.push_str(&format!("  \"home\": \"{}\",\n", json_escape(h))),
        None    => json.push_str("  \"home\": null,\n"),
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
/// Returns `(restored_alive, dropped_dead, restored_foreground_app_id)`. The
/// foreground app-id (if it survived) is returned for the caller to seed the
/// surface model (task 74 — the legacy `foreground` singleton is gone). A
/// missing file is not an error — returns `Ok((0, 0, None))` so cold daemon
/// starts go through the same call site.
pub fn restore_from(path: &Path) -> Result<(usize, usize, Option<String>)> {
    if !path.exists() {
        return Ok((0, 0, None));
    }
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;

    // Tiny ad-hoc JSON walker — sufficient for our fixed schema. If
    // the schema grows we should pull in serde-json then.
    let entries = parse_apps(&body)?;
    let fg = parse_foreground(&body);
    let home = parse_top_string(&body, "home");

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

    // The fg app-id is returned (not applied) so the caller seeds the surface
    // model; the legacy `foreground` singleton no longer exists (task 74 D).
    let restored_fg = match fg {
        Some(fg_id) if get(&fg_id).is_some() => Some(fg_id),
        Some(fg_id) => {
            log::info!("arbiter: state restore — foreground app {fg_id:?} not alive, fg cleared");
            None
        }
        None => None,
    };

    // Home app-id persists regardless of whether it's currently running
    // (task 57) — the arbiter re-launches it on boot / fall-back.
    if let Some(home_id) = home {
        let _ = set_home(Some(&home_id));
        log::info!("arbiter: state restore — home app = {home_id:?}");
    }

    Ok((alive, dead, restored_fg))
}

/// kill(pid, 0) probes for liveness without delivering a signal.
/// 0 → alive (and we have permission); -1 + ESRCH → dead.
/// `pub` so the task-54 polling backstop in `main.rs` can reuse it.
pub fn pid_alive(pid: i32) -> bool {
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
    parse_top_string(body, "foreground")
}

/// Parse a top-level `"<key>": "..."` string field (or `null`) from the
/// hand-rolled state JSON. Used for `foreground` + `home` (task 57).
fn parse_top_string(body: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let idx = body.find(&needle)?;
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
