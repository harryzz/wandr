//! App registry + home designation + IME height + persistence (task 74 C).
//!
//! Moved out of the binary's `state.rs` into the core [`Store`] so the AMS-style
//! responsibility module (`wart-arbiter-shell`) can own these via `Ctx`, and the
//! Store stays the single source of truth (the design doc's rule). The app
//! registry is not display-scoped — surfaces on a [`crate::DisplayState`]
//! reference its pids — so it lives on the `Store` directly, not per-display.
//!
//! Persistence stays the dep-free hand-rolled JSON it has always been (the
//! arbiter deliberately carries no serde). The *logic* (serialize / parse) lives
//! here; the binary still owns the file IO (read the bytes at boot, write them on
//! the `Effect::Persist` executor).

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};

use crate::Store;

/// Default soft-keyboard occlusion until the IME guest reports its real height
/// (task 68) — lets the keyboard inset work before the first report.
pub const DEFAULT_IME_HEIGHT_PX: u32 = 1200;

/// One running app instance. (Was `state::AppState`.)
#[derive(Clone, Debug)]
pub struct AppState {
    pub app_id: String,
    pub pid: i32,
    /// When the LAUNCH socket-command landed. `Instant` for elapsed math +
    /// `SystemTime` for human-readable timestamps.
    pub launched_at: SystemTime,
    pub launched_mono: Instant,
}

impl Store {
    // ── app registry ─────────────────────────────────────────────────────

    /// Insert or replace the entry for an app-id (N=1 per app-id at MVP).
    pub fn insert_app(&mut self, app: AppState) {
        self.apps.insert(app.app_id.clone(), app);
    }

    /// Remove an app from the registry. The surface/role/focus teardown lives in
    /// the model — pair this with `display_mut(id).remove_surface(pid)`.
    pub fn remove_app(&mut self, app_id: &str) -> Option<AppState> {
        self.apps.remove(app_id)
    }

    pub fn app(&self, app_id: &str) -> Option<&AppState> {
        self.apps.get(app_id)
    }

    pub fn app_by_pid(&self, pid: i32) -> Option<&AppState> {
        self.apps.values().find(|a| a.pid == pid)
    }

    /// Cloned snapshot of every running app (for iteration without holding a
    /// borrow on the Store). Cheap — the map is tens of entries at most.
    pub fn apps_snapshot(&self) -> Vec<AppState> {
        self.apps.values().cloned().collect()
    }

    // ── home designation (task 57) ────────────────────────────────────────

    pub fn home(&self) -> Option<&str> {
        self.home.as_deref()
    }

    /// Set / clear the designated home app-id. Returns the previous value.
    pub fn set_home(&mut self, new: Option<String>) -> Option<String> {
        std::mem::replace(&mut self.home, new)
    }

    // ── IME intrinsic height (task 68) ─────────────────────────────────────

    pub fn ime_height(&self) -> u32 {
        self.ime_height
    }

    pub fn set_ime_height(&mut self, px: u32) {
        self.ime_height = px;
    }

    // ── persistence (task 46 crash-marker; serialize/parse only) ───────────
    //
    // The arbiter owns the (app_id → pid) map but the children are forked by the
    // zygote, so they survive an arbiter restart; without persistence `list`
    // would be empty after a restart even though apps are alive. The foreground
    // app-id is sourced by the caller from the surface model (the model is the
    // single source; this only records it for restore).

    /// Serialize the registry + home + foreground to the hand-rolled JSON shape.
    pub fn to_json(&self, foreground_app_id: Option<&str>) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut json = String::new();
        json.push_str("{\n");
        json.push_str("  \"version\": 1,\n");
        json.push_str(&format!("  \"saved_at_unix\": {now},\n"));
        match foreground_app_id {
            Some(f) => json.push_str(&format!("  \"foreground\": \"{}\",\n", json_escape(f))),
            None => json.push_str("  \"foreground\": null,\n"),
        }
        match self.home() {
            Some(h) => json.push_str(&format!("  \"home\": \"{}\",\n", json_escape(h))),
            None => json.push_str("  \"home\": null,\n"),
        }
        json.push_str("  \"apps\": [");
        let apps = self.apps_snapshot();
        for (i, app) in apps.iter().enumerate() {
            if i > 0 {
                json.push_str(", ");
            }
            let launched = app
                .launched_at
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            json.push_str(&format!(
                "\n    {{\"app_id\": \"{}\", \"pid\": {}, \"launched_at_unix\": {}}}",
                json_escape(&app.app_id),
                app.pid,
                launched,
            ));
        }
        if !apps.is_empty() {
            json.push_str("\n  ");
        }
        json.push_str("],\n");
        // Alarms (Arbiter Inc. 3c). `pending_deliver` is transient — not persisted.
        json.push_str("  \"alarms\": [");
        for (i, a) in self.alarms.iter().enumerate() {
            if i > 0 {
                json.push_str(", ");
            }
            json.push_str(&format!(
                "\n    {{\"app_id\": \"{}\", \"alarm_id\": {}, \"next_fire_ms\": {}, \"repeat_ms\": {}, \"wake_kind\": \"{}\"}}",
                json_escape(&a.app_id),
                a.alarm_id,
                a.next_fire_ms,
                a.repeat_ms,
                a.wake_kind.as_wire(),
            ));
        }
        if !self.alarms.is_empty() {
            json.push_str("\n  ");
        }
        json.push_str("]\n");
        json.push_str("}\n");
        json
    }

    /// Restore the registry + home from the JSON `body`. Each persisted pid is
    /// liveness-checked via `kill(pid, 0)`; survivors are re-inserted, dead ones
    /// dropped + logged. The foreground app-id (if it survived) is RETURNED for
    /// the caller to seed the surface model (the legacy `foreground` singleton no
    /// longer exists). Returns `(restored_alive, dropped_dead, restored_fg)`.
    pub fn restore_from_json(&mut self, body: &str) -> Result<(usize, usize, Option<String>)> {
        let entries = parse_apps(body)?;
        let fg = parse_top_string(body, "foreground");
        let home = parse_top_string(body, "home");

        let mut alive = 0usize;
        let mut dead = 0usize;
        for (app_id, pid, launched_secs) in entries {
            if pid_alive(pid) {
                let launched_at = UNIX_EPOCH + std::time::Duration::from_secs(launched_secs);
                self.insert_app(AppState {
                    app_id,
                    pid,
                    launched_at,
                    launched_mono: Instant::now(), // restart-relative is fine for LIST display
                });
                alive += 1;
            } else {
                log::info!(
                    "arbiter: state restore — pid {} for app {:?} is dead, dropping",
                    pid,
                    app_id,
                );
                dead += 1;
            }
        }

        // The fg app-id is returned (not applied) so the caller seeds the surface
        // model; the legacy `foreground` singleton no longer exists (task 74 D).
        let restored_fg = match fg {
            Some(fg_id) if self.app(&fg_id).is_some() => Some(fg_id),
            Some(fg_id) => {
                log::info!("arbiter: state restore — foreground app {fg_id:?} not alive, fg cleared");
                None
            }
            None => None,
        };

        // Home persists regardless of whether it's running (task 57) — the
        // arbiter re-launches it on boot / fall-back.
        if let Some(home_id) = home {
            log::info!("arbiter: state restore — home app = {home_id:?}");
            self.set_home(Some(home_id));
        }

        // Alarms (Arbiter Inc. 3c) — restore so timed wakes survive a restart.
        for a in parse_alarms(body) {
            self.upsert_alarm(a);
        }

        Ok((alive, dead, restored_fg))
    }
}

/// Parse the `"alarms": [ … ]` array. Mirrors `parse_apps`. `pending_deliver`
/// is transient → restored false.
fn parse_alarms(body: &str) -> Vec<crate::Alarm> {
    let Some(open) = body.find("\"alarms\"") else { return vec![]; };
    let Some(arr_start) = body[open..].find('[') else { return vec![]; };
    let arr_start = open + arr_start + 1;
    let Some(arr_end_rel) = body[arr_start..].find(']') else { return vec![]; };
    let arr = &body[arr_start..arr_start + arr_end_rel];

    let mut out = Vec::new();
    for chunk in arr.split('}') {
        let Some(open) = chunk.find('{') else { continue };
        let entry = &chunk[open + 1..];
        let app_id = pick_str(entry, "app_id").unwrap_or_default();
        if app_id.is_empty() {
            continue;
        }
        out.push(crate::Alarm {
            app_id,
            alarm_id: pick_int(entry, "alarm_id").unwrap_or(0) as u64,
            next_fire_ms: pick_int(entry, "next_fire_ms").unwrap_or(0) as u64,
            repeat_ms: pick_int(entry, "repeat_ms").unwrap_or(0) as u64,
            wake_kind: crate::LaunchKind::from_wire(
                &pick_str(entry, "wake_kind").unwrap_or_default(),
            ),
            pending_deliver: false,
        });
    }
    out
}

/// kill(pid, 0) probes for liveness without delivering a signal. 0 → alive (and
/// we have permission); -1 + ESRCH → dead; EPERM → alive but unsignalable.
/// `pub` so the task-54 polling backstop in the binary can reuse it.
pub fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    let r = unsafe { libc::kill(pid, 0) };
    if r == 0 {
        return true;
    }
    let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    err == libc::EPERM
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn parse_apps(body: &str) -> Result<Vec<(String, i32, u64)>> {
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
        let pid = pick_int(entry, "pid").unwrap_or(0) as i32;
        let launched = pick_int(entry, "launched_at_unix").unwrap_or(0) as u64;
        if !app_id.is_empty() && pid > 0 {
            out.push((app_id, pid, launched));
        }
    }
    Ok(out)
}

/// Parse a top-level `"<key>": "..."` string field (or `null`).
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
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, pid: i32) -> AppState {
        AppState {
            app_id: id.to_string(),
            pid,
            launched_at: UNIX_EPOCH + std::time::Duration::from_secs(1730000000),
            launched_mono: Instant::now(),
        }
    }

    #[test]
    fn json_round_trips_apps_home_fg() {
        let mut s = Store::new();
        s.insert_app(app("com.example.a", 100));
        s.insert_app(app("war.launcher", 101));
        s.set_home(Some("war.launcher".into()));
        let json = s.to_json(Some("com.example.a"));

        // Restore into a fresh store. pid_alive will drop pids that aren't live;
        // pid 1 (init) is always alive, so re-key the JSON onto live pids.
        let json = json.replace("100", "1").replace("101", "1");
        let mut t = Store::new();
        let (alive, _dead, fg) = t.restore_from_json(&json).unwrap();
        assert_eq!(alive, 2);
        assert_eq!(t.home(), Some("war.launcher"));
        assert_eq!(fg.as_deref(), Some("com.example.a"));
    }
}
