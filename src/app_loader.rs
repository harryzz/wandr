//! App loader — task 35 steps 1 + 5.
//!
//! Uniform interface for resolving a `Component` + skiko-wired `Linker`
//! from one of three sources: an installed app (`AppRef::Installed`,
//! re-verifies + self-heals the AOT cache), a dev-machine `.cwasm`/`.wasm`
//! path search, or APK-asset cwasm bytes.
//!
//! The loader does NOT build `HostState` and does NOT call `instantiate`.
//! Callers bring a `Store<HostState>` to `LoadedApp::instantiate`.
//!
//! See `tasks/35-app-install.md` for scope.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Engine, Store};

use crate::app_installer::{
    engine_compatibility_hash_hex, format_cache_key, sha256_hex, ComponentCacheEntry,
};
use crate::bindings;
use crate::HostState;

/// What the caller wants to load.
pub enum AppRef<'a> {
    /// Installed app under the loader root — `<root>/<app_id>/<version>/`.
    /// If `version` is `None`, the loader picks the lexicographically
    /// highest version dir present.
    Installed { app_id: &'a str, version: Option<&'a str> },
    /// Dev shortcut: try `.cwasm` (AOT) and `.wasm` (JIT) paths in order,
    /// take the first that loads. Subsumes both today's `argv[1]` flow
    /// and `find_cwasm_on_filesystem`'s candidate list.
    DevCwasm { candidates: &'a [&'a Path] },
    /// Dev shortcut: AOT cwasm bytes already in memory (APK asset).
    DevAsset { bytes: &'a [u8] },
}

/// A loaded component ready to instantiate. The `linker` already carries
/// `wasmtime_wasi::p2::add_to_linker_sync` (WASI imports) and
/// `bindings::SkikoUi::add_to_linker` (skiko-gfx WIT host).
pub struct LoadedApp {
    /// Human-readable origin for logs, e.g. `"cwasm:/data/local/tmp/skiko-component.cwasm"`.
    pub source_label: String,
    entry: Component,
    linker: Linker<HostState>,
}

impl LoadedApp {
    pub fn instantiate(&self, store: &mut Store<HostState>) -> Result<bindings::SkikoUi> {
        bindings::SkikoUi::instantiate(store, &self.entry, &self.linker)
            .map_err(|e| anyhow!("SkikoUi::instantiate failed: {e:#}"))
    }
}

pub trait AppLoader {
    fn load(&self, engine: &Engine, r: AppRef<'_>) -> Result<LoadedApp>;
}

/// Default loader. `root` is reserved for `AppRef::Installed` (task 35
/// step 5); the dev variants ignore it.
pub struct WartLoader {
    pub root: PathBuf,
}

/// Convenience entry point. Default root is `/data/wart/apps` (the
/// scoped registry root). Smoke / dev override via `WART_APPS_ROOT`
/// env var — useful on un-sepolicy'd devices where `/data/wart/` is
/// not writable from a `su` shell.
pub fn default_for_target() -> WartLoader {
    let root = std::env::var("WART_APPS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/data/wart/apps"));
    WartLoader { root }
}

impl AppLoader for WartLoader {
    fn load(&self, engine: &Engine, r: AppRef<'_>) -> Result<LoadedApp> {
        let (entry, source_label) = match r {
            AppRef::Installed { app_id, version } => {
                load_installed(engine, &self.root, app_id, version)?
            }
            AppRef::DevCwasm { candidates } => load_dev_path(engine, candidates)?,
            AppRef::DevAsset { bytes } => load_dev_asset(engine, bytes)?,
        };
        let linker = build_linker(engine)?;
        Ok(LoadedApp { source_label, entry, linker })
    }
}

/// Load from `<root>/<app_id>/<version>/`. Reads `cache-key.toml`,
/// recomputes the engine-compat + per-component wasm hashes; on any
/// drift (host upgrade, manifest mutation, file corruption) re-calls
/// `Engine::precompile_component`, rewrites `cache/<name>.cwasm`, and
/// re-stamps `cache-key.toml`. Then `deserialize_file`s the cwasm.
///
/// Single-component apps only — bails on multi-component (link.wac
/// composition is deferred to `tasks/scope-cross-app-deps.md`).
fn load_installed(
    engine: &Engine,
    root: &Path,
    app_id: &str,
    version: Option<&str>,
) -> Result<(Component, String)> {
    let app_dir = root.join(app_id);
    if !app_dir.is_dir() {
        bail!("installed: app dir not found: {}", app_dir.display());
    }
    let version_str = match version {
        Some(v) => v.to_string(),
        None => pick_latest_version(&app_dir)?,
    };
    let install_dir = app_dir.join(&version_str);
    if !install_dir.is_dir() {
        bail!("installed: install dir not found: {}", install_dir.display());
    }

    let key_path = install_dir.join("cache-key.toml");
    let key_src = fs::read_to_string(&key_path)
        .with_context(|| format!("read {}", key_path.display()))?;
    let key: toml::Value = key_src.parse()
        .with_context(|| format!("parse {}", key_path.display()))?;
    let stored_engine_hash = key.get("engine_config_hash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("{}: missing engine_config_hash", key_path.display()))?
        .to_string();
    let components_tbl = key.get("components").and_then(|v| v.as_table())
        .ok_or_else(|| anyhow!("{}: missing [components] table", key_path.display()))?;
    if components_tbl.is_empty() {
        bail!("{}: [components] is empty", key_path.display());
    }
    if components_tbl.len() > 1 {
        bail!(
            "{}: {} components — loader only supports single-component apps. \
             Multi-component composition is the scope-cross-app-deps task.",
            key_path.display(), components_tbl.len(),
        );
    }
    let (component_name, entry_val) = components_tbl.iter().next().unwrap();
    let component_name = component_name.clone();
    let stored_wasm_sha = entry_val.get("wasm_sha256").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("{}: components.{component_name}.wasm_sha256 missing", key_path.display()))?
        .to_string();

    let wasm_path = install_dir.join("components").join(format!("{component_name}.wasm"));
    let cwasm_path = install_dir.join("cache").join(format!("{component_name}.cwasm"));
    let wasm_bytes = fs::read(&wasm_path)
        .with_context(|| format!("read {}", wasm_path.display()))?;
    let current_wasm_sha = sha256_hex(&wasm_bytes);
    let current_engine_hash = engine_compatibility_hash_hex(engine);

    let engine_match = current_engine_hash == stored_engine_hash;
    let wasm_match = current_wasm_sha == stored_wasm_sha;
    let cache_present = cwasm_path.exists();

    if !engine_match || !wasm_match || !cache_present {
        log::info!(
            "loader: cache drift for {app_id} {version_str} \
             (engine={engine_match} wasm={wasm_match} cwasm_present={cache_present}) — re-precompiling",
        );
        let cwasm_bytes = engine.precompile_component(&wasm_bytes)
            .map_err(|e| anyhow!("precompile_component({component_name}): {e:#}"))?;
        fs::write(&cwasm_path, &cwasm_bytes)
            .with_context(|| format!("write {}", cwasm_path.display()))?;
        let new_cwasm_sha = sha256_hex(&cwasm_bytes);
        let new_key = format_cache_key(engine, &[(
            component_name.clone(),
            ComponentCacheEntry {
                wasm_sha256: current_wasm_sha,
                cwasm_sha256: new_cwasm_sha,
            },
        )]);
        fs::write(&key_path, new_key)
            .with_context(|| format!("write {}", key_path.display()))?;
        log::info!("loader: re-stamped {}", key_path.display());
    } else {
        log::debug!("loader: cache fresh for {app_id} {version_str}");
    }

    let component = unsafe { Component::deserialize_file(engine, &cwasm_path) }
        .map_err(|e| anyhow!("Component::deserialize_file({}): {e:#}", cwasm_path.display()))?;
    let label = format!("installed:{app_id}:{version_str}:{component_name}");
    Ok((component, label))
}

/// Pick the lexicographically highest subdirectory of `app_dir`. Works
/// for `MAJOR.MINOR.PATCH` versions; not a proper semver sort (e.g.
/// `0.10.0` < `0.2.0` lexicographically). When that bites, callers
/// should pass `version: Some(...)` explicitly.
fn pick_latest_version(app_dir: &Path) -> Result<String> {
    let mut versions: Vec<String> = Vec::new();
    for entry in fs::read_dir(app_dir)
        .with_context(|| format!("read_dir {}", app_dir.display()))?
    {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                versions.push(name.to_string());
            }
        }
    }
    versions.sort();
    versions.pop().ok_or_else(|| anyhow!("no versions installed under {}", app_dir.display()))
}

/// `.cwasm` → AOT `deserialize_file`; anything else → JIT `from_file`.
/// First successful load wins.
fn load_dev_path(engine: &Engine, candidates: &[&Path]) -> Result<(Component, String)> {
    if candidates.is_empty() {
        bail!("AppRef::DevCwasm: empty candidates list");
    }
    let mut last_err: Option<anyhow::Error> = None;
    for path in candidates {
        let is_cwasm = path.extension().map_or(false, |e| e == "cwasm");
        let r = if is_cwasm {
            unsafe { Component::deserialize_file(engine, path) }
        } else {
            Component::from_file(engine, path)
        };
        match r {
            Ok(c) => {
                let label = format!(
                    "{}:{}",
                    if is_cwasm { "cwasm" } else { "wasm" },
                    path.display(),
                );
                return Ok((c, label));
            }
            Err(e) => {
                log::debug!("app_loader: {} miss: {e}", path.display());
                last_err = Some(e.into());
            }
        }
    }
    let detail = last_err
        .map(|e| format!("last error: {e:#}"))
        .unwrap_or_default();
    bail!("no candidate loaded out of {} path(s); {detail}", candidates.len())
}

fn load_dev_asset(engine: &Engine, bytes: &[u8]) -> Result<(Component, String)> {
    let entry = unsafe { Component::deserialize(engine, bytes) }
        .map_err(|e| anyhow!("Component::deserialize (asset, {} bytes): {e:#}", bytes.len()))?;
    Ok((entry, format!("asset:{}B", bytes.len())))
}

fn build_linker(engine: &Engine) -> Result<Linker<HostState>> {
    let mut linker: Linker<HostState> = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
        .map_err(|e| anyhow!("wasmtime_wasi::p2::add_to_linker_sync: {e:#}"))?;
    bindings::SkikoUi::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |s| s)
        .map_err(|e| anyhow!("SkikoUi::add_to_linker: {e:#}"))?;
    Ok(linker)
}
