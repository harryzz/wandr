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
    engine_compatibility_hash_hex, format_cache_key, parse_resolved_deps_from_key,
    sha256_hex, ComponentCacheEntry,
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

/// A loaded component ready to instantiate.
///
/// The `linker` is built fresh in `instantiate()` rather than cached on
/// the struct so the consumer's deps (loaded into the Store at
/// instantiation time) can register their exports as proxy entries
/// into the linker. wasmtime's `Linker::instantiate` takes `&self`, so
/// any wiring that depends on a live Store must be done *during*
/// `instantiate()`, not at `load()` time.
pub struct LoadedApp {
    /// Human-readable origin for logs, e.g. `"cwasm:/data/local/tmp/skiko-component.cwasm"`.
    pub source_label: String,
    entry: Component,
    /// Cloned `Engine` (cheap — Arc-backed) so `instantiate()` can build
    /// a fresh `Linker::new(engine)` without needing the caller to
    /// re-pass it.
    engine: Engine,
    /// Same-Store deps resolved from `[dependencies_resolved]`. Empty
    /// for `DevCwasm` / `DevAsset` (task-35-style) loads. Each entry
    /// is deserialized but not yet instantiated; instantiation happens
    /// inside `instantiate()` against the caller's Store.
    deps: Vec<LoadedDep>,
}

/// One resolved + deserialized same-Store dep.
struct LoadedDep {
    /// Local alias from the consumer's manifest (LHS in `[dependencies]`).
    /// Used for log lines + error messages.
    name:      String,
    /// WIT-qualified interface name (e.g. `"war:markdown/renderer@0.1.0"`).
    /// Dispatched by `wire_dep_into_linker` to pick the right host-side
    /// bindgen module.
    interface: String,
    component: Component,
}

impl LoadedApp {
    pub fn instantiate(&self, store: &mut Store<HostState>) -> Result<bindings::SkikoUi> {
        let mut linker: Linker<HostState> = Linker::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
            .map_err(|e| anyhow!("wasmtime_wasi::p2::add_to_linker_sync: {e:#}"))?;
        bindings::SkikoUi::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |s| s)
            .map_err(|e| anyhow!("SkikoUi::add_to_linker: {e:#}"))?;

        for dep in &self.deps {
            wire_dep_into_linker(&mut linker, store, dep)?;
        }

        bindings::SkikoUi::instantiate(store, &self.entry, &linker)
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
        let (entry, source_label, deps) = match r {
            AppRef::Installed { app_id, version } => {
                let (entry, label, deps) =
                    load_installed(engine, &self.root, app_id, version)?;
                (entry, label, deps)
            }
            AppRef::DevCwasm { candidates } => {
                let (entry, label) = load_dev_path(engine, candidates)?;
                (entry, label, Vec::new())
            }
            AppRef::DevAsset { bytes } => {
                let (entry, label) = load_dev_asset(engine, bytes)?;
                (entry, label, Vec::new())
            }
        };
        Ok(LoadedApp { source_label, entry, engine: engine.clone(), deps })
    }
}

/// Load from `<root>/apps/<app_id>/<version>/`. Reads `cache-key.toml`,
/// recomputes the engine-compat + per-component wasm hashes; on any
/// drift (host upgrade, manifest mutation, file corruption) re-calls
/// `Engine::precompile_component`, rewrites `cache/<name>.cwasm`, and
/// re-stamps `cache-key.toml`. Then `deserialize_file`s the cwasm.
///
/// Single-component apps only — bails on multi-component (link.wac
/// composition is deferred to `tasks/36-cross-app-deps.md` step 5).
///
/// Task 36 layout: `AppRef::Installed` always targets the `apps/`
/// subtree; system components are reached via the consumer's resolved
/// deps, not directly through `Installed`.
fn load_installed(
    engine: &Engine,
    root: &Path,
    app_id: &str,
    version: Option<&str>,
) -> Result<(Component, String, Vec<LoadedDep>)> {
    let app_dir = root.join("apps").join(app_id);
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
        // Preserve the existing `[dependencies_resolved]` section
        // verbatim — engine/wasm drift doesn't invalidate dep entries
        // (those are checked separately in step 5).
        let preserved_deps = parse_resolved_deps_from_key(&key);
        let new_key = format_cache_key(
            engine,
            &[(
                component_name.clone(),
                ComponentCacheEntry {
                    wasm_sha256: current_wasm_sha,
                    cwasm_sha256: new_cwasm_sha,
                },
            )],
            &preserved_deps,
        );
        fs::write(&key_path, new_key)
            .with_context(|| format!("write {}", key_path.display()))?;
        log::info!("loader: re-stamped {}", key_path.display());
    } else {
        log::debug!("loader: cache fresh for {app_id} {version_str}");
    }

    let component = unsafe { Component::deserialize_file(engine, &cwasm_path) }
        .map_err(|e| anyhow!("Component::deserialize_file({}): {e:#}", cwasm_path.display()))?;
    let label = format!("installed:{app_id}:{version_str}:{component_name}");

    // Task 36 step 5 — load same-Store deps. Reads `[dependencies_resolved]`
    // from cache-key.toml (parsed above as `key`), looks up each dep's
    // cwasm under `<root>/<kind_dir>/<id>/<version>/cache/<entry>.cwasm`,
    // verifies the dep's on-disk wasm sha256 still matches the stored
    // value (warn-and-load on mismatch; full re-precompile-the-dep is a
    // follow-up), and deserializes the dep's cwasm for instantiation at
    // `LoadedApp::instantiate` time.
    let deps = load_dep_components(engine, root, &key)?;

    Ok((component, label, deps))
}

fn load_dep_components(
    engine: &Engine,
    root: &Path,
    key: &toml::Value,
) -> Result<Vec<LoadedDep>> {
    let mut deps: Vec<LoadedDep> = Vec::new();
    let Some(tbl) = key.get("dependencies_resolved").and_then(|v| v.as_table()) else {
        return Ok(deps);
    };
    for (name, val) in tbl {
        let Some(entry) = val.as_table() else { continue };
        let kind = entry.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let id = entry.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let version = entry.get("version").and_then(|v| v.as_str()).unwrap_or("");
        let stored_sha = entry.get("wasm_sha256").and_then(|v| v.as_str());
        let interface = entry.get("interface").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!(
                "dependency {name}: missing interface in cache-key.toml — \
                 same-store composition requires the WIT-qualified interface name"
            ))?
            .to_string();

        // Host deps are satisfied by `wasmtime_wasi::p2::add_to_linker_sync`
        // + `bindings::SkikoUi::add_to_linker` already applied in
        // `LoadedApp::instantiate`. No per-load wiring needed.
        if kind == "host" {
            log::debug!("loader: host dep `{name}` ({interface}) — already in linker");
            continue;
        }

        let kind_dir = match kind {
            "system" => "system-apps",
            "app"    => "apps",
            other    => bail!("dependency {name}: unknown kind {other:?}"),
        };
        let dep_dir = root.join(kind_dir).join(id).join(version);
        if !dep_dir.is_dir() {
            bail!(
                "dependency {name}: missing install dir {} \
                 (was the dep uninstalled after consumer install?)",
                dep_dir.display(),
            );
        }

        // Verify the dep's on-disk wasm hasn't drifted since consumer
        // install. Full re-precompile (touching the DEP's own
        // cache-key.toml) is a follow-up; for now, warn-and-load.
        if let Some(expected) = stored_sha {
            let (_dep_wasm_path, current) = hash_dep_wasm(&dep_dir).with_context(|| {
                format!("dependency {name}: failed to hash wasm under {}", dep_dir.display())
            })?;
            if current != expected {
                log::warn!(
                    "loader: dependency `{name}` wasm sha256 drifted \
                     (stored={expected}, current={current}) — loading anyway. \
                     Re-install the consumer to refresh cache-key.toml."
                );
            }
        }

        let cwasm_path = first_cwasm(&dep_dir.join("cache")).with_context(|| {
            format!("dependency {name}: no cwasm under {}/cache", dep_dir.display())
        })?;
        let component = unsafe { Component::deserialize_file(engine, &cwasm_path) }
            .map_err(|e| anyhow!(
                "dependency {name}: Component::deserialize_file({}): {e:#}", cwasm_path.display()
            ))?;
        log::info!("loader: loaded dep `{name}` ({interface}) from {}", cwasm_path.display());
        deps.push(LoadedDep { name: name.clone(), interface, component });
    }
    Ok(deps)
}

fn hash_dep_wasm(dep_dir: &Path) -> Result<(PathBuf, String)> {
    let components_dir = dep_dir.join("components");
    for entry in fs::read_dir(&components_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("wasm") {
            let bytes = fs::read(&path)?;
            return Ok((path, sha256_hex(&bytes)));
        }
    }
    bail!("no .wasm under {}", components_dir.display())
}

fn first_cwasm(cache_dir: &Path) -> Result<PathBuf> {
    for entry in fs::read_dir(cache_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("cwasm") {
            return Ok(path);
        }
    }
    bail!("no .cwasm under {}", cache_dir.display())
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

/// Same-Store composition glue — task 36 step 5.
///
/// Instantiates the dep into the consumer's `Store`, then registers
/// the dep's exports as proxy entries in the consumer's `Linker`. The
/// consumer's subsequent `SkikoUi::instantiate` finds its
/// `import war:markdown/renderer` satisfied by the proxy that
/// delegates back to the dep instance.
///
/// First concrete dep wired by name. When a second runtime-bundled
/// system component lands, refactor into a `(interface) → impl`
/// registry pattern instead of this match.
fn wire_dep_into_linker(
    linker: &mut Linker<HostState>,
    store: &mut Store<HostState>,
    dep: &LoadedDep,
) -> Result<()> {
    match dep.interface.as_str() {
        "war:markdown/renderer@0.1.0" => wire_markdown_dep(linker, store, dep),
        other => bail!(
            "dependency `{}`: interface {other:?} is not yet wired in wart-host. \
             Supported interfaces this iteration: war:markdown/renderer@0.1.0. \
             See tasks/36-cross-app-deps.md.",
            dep.name,
        ),
    }
}

fn wire_markdown_dep(
    linker: &mut Linker<HostState>,
    store: &mut Store<HostState>,
    dep: &LoadedDep,
) -> Result<()> {
    use crate::markdown_bindings::{exports::war::markdown::renderer as md_exports, RendererWorld};

    // Build a dep-local linker — markdown only imports WASI, no skiko.
    let mut dep_linker: Linker<HostState> = Linker::new(&store.engine().clone());
    wasmtime_wasi::p2::add_to_linker_sync(&mut dep_linker)
        .map_err(|e| anyhow!("dep linker wasi add: {e:#}"))?;

    let world = RendererWorld::instantiate(&mut *store, &dep.component, &dep_linker)
        .map_err(|e| anyhow!("instantiate dep `{}`: {e:#}", dep.name))?;
    // `Guest` derives `Clone` and holds only `wasmtime::component::Func`
    // handles (instance-indices, not borrows), so cloning the accessor
    // lets us drop `world` while keeping the call site usable from a
    // `'static` linker closure.
    let renderer = world.war_markdown_renderer().clone();
    log::info!(
        "loader: dep `{}` instantiated; wiring war:markdown/renderer@0.1.0 \
         → consumer linker",
        dep.name,
    );

    // Stash the per-call entry point in the consumer's linker. Each
    // call forwards `(source)` to the dep instance's `render` and
    // returns the resulting `Document`.
    let mut inst = linker.instance("war:markdown/renderer@0.1.0")
        .map_err(|e| anyhow!("linker.instance(war:markdown/renderer@0.1.0): {e:#}"))?;
    inst.func_wrap(
        "render",
        move |mut store, (source,): (String,)| -> wasmtime::Result<(md_exports::Document,)> {
            let doc = renderer.call_render(&mut store, &source)?;
            Ok((doc,))
        },
    ).map_err(|e| anyhow!("linker proxy `render`: {e:#}"))?;
    Ok(())
}
