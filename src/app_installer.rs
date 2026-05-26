//! App installer — task 35 step 4.
//!
//! Reads a `.warpkg` bundle (a directory containing `package.toml` +
//! `components/*.wasm` + optional `assets/`), AOT-precompiles each
//! component for the device via `Engine::precompile_component`, and
//! writes the install dir at `<root>/<app_id>/<version>/` with a
//! `cache-key.toml` for the loader's invalidation check.
//!
//! Layout written:
//! ```text
//! <root>/<app_id>/<version>/
//!   package.toml             # copied verbatim
//!   components/<name>.wasm   # one per [components] entry
//!   cache/<name>.cwasm       # AOT artefact for this device's engine
//!   assets/                  # copied verbatim if bundle has one
//!   cache-key.toml           # wasmtime_version + engine_config_hash + per-component sha256
//! ```
//!
//! Step 5 (loader) re-reads `cache-key.toml` to decide whether to use
//! the cached cwasm or re-call `precompile_component`.
//!
//! See `tasks/35-app-install.md`.

use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use wasmtime::Engine;

/// Wasmtime version pinned in `Cargo.toml`. Diagnostic field in
/// `cache-key.toml`; the *cache-invalidation* signal is
/// `engine_config_hash` (which folds in the wasmtime version via
/// `precompile_compatibility_hash`).
const WASMTIME_PINNED_VERSION: &str = "44";

/// Input to `install` — an unpacked `.warpkg` directory.
pub struct PackageBundle<'a> {
    pub dir: &'a Path,
}

impl<'a> PackageBundle<'a> {
    pub fn from_dir(dir: &'a Path) -> Self { Self { dir } }
}

/// What the registry / caller learns from a successful install.
pub struct InstalledApp {
    pub app_id: String,
    pub version: String,
    pub install_dir: PathBuf,
}

pub trait AppInstaller {
    fn install(&self, engine: &Engine, bundle: PackageBundle<'_>) -> Result<InstalledApp>;
}

/// Default installer. Writes under `<root>/<app_id>/<version>/`.
pub struct WartInstaller {
    pub root: PathBuf,
}

pub fn default_for_target() -> WartInstaller {
    let root = std::env::var("WART_APPS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/data/wart/apps"));
    WartInstaller { root }
}

impl AppInstaller for WartInstaller {
    fn install(&self, engine: &Engine, bundle: PackageBundle<'_>) -> Result<InstalledApp> {
        let manifest = parse_manifest(bundle.dir)?;
        log::info!(
            "installer: {} v{} ({} component(s)) world={}",
            manifest.app_id, manifest.version, manifest.components.len(), manifest.world,
        );

        // Q5b: signing format not picked yet — placeholder always Ok.
        verify_signature(&bundle)?;

        let install_dir = self.root.join(&manifest.app_id).join(&manifest.version);
        if install_dir.exists() {
            log::warn!("installer: {} already exists — overwriting", install_dir.display());
        }
        let components_dir = install_dir.join("components");
        let cache_dir = install_dir.join("cache");
        fs::create_dir_all(&components_dir)
            .with_context(|| format!("create_dir_all {}", components_dir.display()))?;
        fs::create_dir_all(&cache_dir)
            .with_context(|| format!("create_dir_all {}", cache_dir.display()))?;

        copy_file(&bundle.dir.join("package.toml"), &install_dir.join("package.toml"))?;
        let assets_src = bundle.dir.join("assets");
        if assets_src.is_dir() {
            copy_dir_recursive(&assets_src, &install_dir.join("assets"))?;
        }

        let mut cache_entries: Vec<(String, ComponentCacheEntry)> = Vec::new();
        for (name, rel_path) in &manifest.components {
            let wasm_src = bundle.dir.join(rel_path);
            let wasm_dst = components_dir.join(format!("{name}.wasm"));
            let wasm_bytes = fs::read(&wasm_src)
                .with_context(|| format!("read {}", wasm_src.display()))?;
            fs::write(&wasm_dst, &wasm_bytes)
                .with_context(|| format!("write {}", wasm_dst.display()))?;
            let wasm_sha = sha256_hex(&wasm_bytes);

            log::info!("installer: AOT-compiling {name} ({} bytes) …", wasm_bytes.len());
            let cwasm_bytes = engine.precompile_component(&wasm_bytes)
                .map_err(|e| anyhow!("precompile_component({name}): {e:#}"))?;
            let cwasm_path = cache_dir.join(format!("{name}.cwasm"));
            fs::write(&cwasm_path, &cwasm_bytes)
                .with_context(|| format!("write {}", cwasm_path.display()))?;
            let cwasm_sha = sha256_hex(&cwasm_bytes);
            log::info!(
                "installer: {name}.cwasm {} bytes → {}",
                cwasm_bytes.len(), cwasm_path.display(),
            );
            cache_entries.push((
                name.clone(),
                ComponentCacheEntry { wasm_sha256: wasm_sha, cwasm_sha256: cwasm_sha },
            ));
        }

        let key_doc = format_cache_key(engine, &cache_entries);
        fs::write(install_dir.join("cache-key.toml"), key_doc)
            .with_context(|| format!("write cache-key.toml at {}", install_dir.display()))?;

        Ok(InstalledApp {
            app_id: manifest.app_id,
            version: manifest.version,
            install_dir,
        })
    }
}

struct Manifest {
    app_id: String,
    version: String,
    world: String,
    components: Vec<(String, PathBuf)>,
}

pub(crate) struct ComponentCacheEntry {
    pub wasm_sha256: String,
    pub cwasm_sha256: String,
}

fn parse_manifest(bundle_dir: &Path) -> Result<Manifest> {
    let pkg_path = bundle_dir.join("package.toml");
    let src = fs::read_to_string(&pkg_path)
        .with_context(|| format!("read {}", pkg_path.display()))?;
    let doc: toml::Value = src.parse()
        .with_context(|| format!("parse {}", pkg_path.display()))?;
    let app_id = doc.get("app_id").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("package.toml: missing app_id"))?
        .to_string();
    let version = doc.get("version").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("package.toml: missing version"))?
        .to_string();
    let world = doc.get("world").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("package.toml: missing world"))?
        .to_string();
    let components_tbl = doc.get("components").and_then(|v| v.as_table())
        .ok_or_else(|| anyhow!("package.toml: missing [components] table"))?;
    if components_tbl.is_empty() {
        bail!("package.toml: [components] is empty");
    }
    let mut components: Vec<(String, PathBuf)> = Vec::new();
    for (name, val) in components_tbl {
        let rel = val.as_str().ok_or_else(|| {
            anyhow!("package.toml: components.{name} must be a string path")
        })?;
        components.push((name.clone(), PathBuf::from(rel)));
    }
    Ok(Manifest { app_id, version, world, components })
}

pub(crate) fn format_cache_key(engine: &Engine, entries: &[(String, ComponentCacheEntry)]) -> String {
    let cfg_hash = engine_compatibility_hash_hex(engine);
    let mut out = String::new();
    out.push_str(&format!("wasmtime_version = \"{WASMTIME_PINNED_VERSION}\"\n"));
    out.push_str(&format!("engine_config_hash = \"{cfg_hash}\"\n\n"));
    for (name, entry) in entries {
        out.push_str(&format!("[components.{name}]\n"));
        out.push_str(&format!("wasm_sha256  = \"{}\"\n", entry.wasm_sha256));
        out.push_str(&format!("cwasm_sha256 = \"{}\"\n\n", entry.cwasm_sha256));
    }
    out
}

/// wasmtime's `precompile_compatibility_hash` returns an opaque `impl Hash`
/// that covers every compile flag + the wasmtime version. We feed it
/// through Sha256 for a stable hex fingerprint.
pub(crate) fn engine_compatibility_hash_hex(engine: &Engine) -> String {
    struct Sha256Hasher(Sha256);
    impl Hasher for Sha256Hasher {
        fn finish(&self) -> u64 { 0 }
        fn write(&mut self, bytes: &[u8]) { self.0.update(bytes); }
    }
    let mut h = Sha256Hasher(Sha256::new());
    engine.precompile_compatibility_hash().hash(&mut h);
    format!("sha256:{}", hex_lower(&h.0.finalize()))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut d = Sha256::new();
    d.update(bytes);
    format!("sha256:{}", hex_lower(&d.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn verify_signature(_bundle: &PackageBundle<'_>) -> Result<()> { Ok(()) }

fn copy_file(src: &Path, dst: &Path) -> Result<()> {
    fs::copy(src, dst)
        .map(|_| ())
        .with_context(|| format!("copy {} → {}", src.display(), dst.display()))
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            copy_file(&from, &to)?;
        }
    }
    Ok(())
}
