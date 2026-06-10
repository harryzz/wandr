---
name: task-38-wandrpkg-assets
description: Task 38 done 2026-05-26 — wandrpkg now supports bundled data files (assets/), surfaced to guests via the my:skiko-gfx/assets.read host WIT verb; WASI fs preopen at /assets also wired but unused for now (no Kotlin/Wasm filesystem APIs)
metadata: 
  node_type: memory
  type: project
  originSessionId: d9451151-9116-4c95-a45d-8758673104ce
---

Task 38 done 2026-05-26. Warpkg now supports bundled data files (`assets/` at wandrpkg root); installer auto-copies them to `<install_dir>/assets/`; runtime exposes via a new `my:skiko-gfx/assets.read(name) -> option<list<u8>>` WIT verb.

**Why custom WIT instead of WASI filesystem read:**
- WASI fs preopen IS wired (`WasiCtxBuilder::preopened_dir(<install_dir>/assets, "/assets", READ, READ)` in both standalone.rs and run_once.rs) but Kotlin/Wasm 2.4 stdlib ships NO fd_read/path_open wrappers (no `kotlin.io.path.Path` impl on wasi). Hand-writing 100+ LoC of preview1 file ops in Kotlin to make a 30-line custom WIT redundant didn't pencil out.
- The custom WIT is host-driven (matches the design rule from `docs/architecture-host-guest-boundary.md`), source-agnostic (could swap to APK assets / network later without touching guest), and has a clean path-safety guard in `wandr-host/src/assets_impl.rs` (rejects `..`, absolute paths, empty names).
- Both stay wired. Future Kotlin stdlib with file APIs gets to use `/assets` directly without host changes.

**Surface added:**

- `wit/skiko-gfx.wit` — new `interface assets { read: ...; }` + `import assets;` in `skiko-ui` world. NOT synced to skiko fork (skiko hand-writes Kotlin bindings; the canonical WIT is wandr-host source-of-truth).
- `wandr-host/src/assets_impl.rs` — `impl Host for HostState` with path-safety guard.
- `wandr-host/src/{lib,app_loader,standalone,run_once}.rs` — HostState gains `assets_dir: Option<PathBuf>`; LoadedApp gains `install_dir: Option<PathBuf>` + `assets_dir()` accessor; both entry points wire preopen AND populate HostState.
- `wandr-app/src/wasmWasiMain/kotlin/AssetsImports.kt` — hand-written Kotlin binding (`readAsset(name): ByteArray?`).
- `wandr-app/src/wasmWasiMain/kotlin/MarkdownCard.kt` — reads `assets/demo.md` via `readAsset`, falls back to inline FALLBACK_SOURCE on missing/null.
- `wandr-app/assets/demo.md` — first real shipped asset file.

**Out of scope (still):**

- Per-asset hashing in cache-key.toml (assets don't gate cwasm reuse).
- Writable `/data` preopen for app state — separate `preopened_dir` call when first needed.
- Manifest-declared `[assets]` (table or path) — auto-detection works for now; declare-it-to-validate-it can come later if asset declarations need metadata (mime, locale, density variants).
- Asset streaming for large payloads — `read()` returns the whole `list<u8>`. Fine up to a few MB; add a streaming variant when first needed.

**Related:**
[[task-36-step-7-pending]], [[adb-push-dir-nesting-gotcha]] (bites asset dirs too — `rm -rf` dest before re-pushing wandrpkg), [[wit-bindgen-no-kotlin-generator]] (why AssetsImports.kt is hand-written).
