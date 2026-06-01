//! Registry / home / IME-height / persistence — thin shim over the core
//! [`Store`] (task 74 C1).
//!
//! The actual storage moved into `wart-arbiter-core` so the responsibility
//! modules can own it via `Ctx` and the Store stays the single source of truth.
//! This shim keeps the binary's existing `state::*` call sites working unchanged
//! while the storage lives in `core_store()`; the orchestration handlers that use
//! it move into `wart-arbiter-shell` in C2, at which point this shim is deleted.
//!
//! File IO for persistence stays here (the binary owns IO); the serialize/parse
//! logic lives in core (`Store::to_json` / `Store::restore_from_json`).

use std::path::Path;

use anyhow::{Context, Result};

pub use wart_arbiter_core::{pid_alive, AppState};

use crate::core_store;

fn store() -> std::sync::MutexGuard<'static, wart_arbiter_core::Store> {
    core_store().lock().unwrap_or_else(|e| e.into_inner())
}

// ── app registry ──────────────────────────────────────────────────────

pub fn insert(app: AppState) {
    store().insert_app(app);
}

pub fn get(app_id: &str) -> Option<AppState> {
    store().app(app_id).cloned()
}

pub fn snapshot() -> Vec<AppState> {
    store().apps_snapshot()
}

// ── home designation (task 57) ──────────────────────────────────────────

pub fn current_home() -> Option<String> {
    store().home().map(|s| s.to_string())
}

pub fn set_home(new: Option<&str>) -> Option<String> {
    store().set_home(new.map(|s| s.to_string()))
}

// ── persistence (file IO here; serialize/parse in core) ─────────────────

/// Write the current state to `path` atomically (`<path>.tmp` → rename). The
/// foreground app-id is sourced by the caller from the surface model.
pub fn save_to(path: &Path, foreground_app_id: Option<&str>) -> Result<()> {
    let json = store().to_json(foreground_app_id);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} → {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Restore from `path`. A missing file is not an error — returns
/// `Ok((0, 0, None))`. Returns `(alive, dead, restored_foreground_app_id)`.
pub fn restore_from(path: &Path) -> Result<(usize, usize, Option<String>)> {
    if !path.exists() {
        return Ok((0, 0, None));
    }
    let body = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    store().restore_from_json(&body)
}
