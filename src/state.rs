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
