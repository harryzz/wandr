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
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime};

/// Which app-id (if any) is currently foreground. The arbiter
/// guarantees at most one. `None` = nothing in foreground (cold
/// arbiter, or last launch was demoted without a successor).
fn foreground() -> &'static Mutex<Option<String>> {
    static FG: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    FG.get_or_init(|| Mutex::new(None))
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
