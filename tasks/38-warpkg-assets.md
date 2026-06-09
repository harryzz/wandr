# Task 38 — asset / data wiring for wandrpkg

> **Status:** ✅ device-verified 2026-05-26. wandr-app's MarkdownCard
> reads `assets/demo.md` from the install dir via the new
> `my:skiko-gfx/assets.read` host WIT verb, hands it to the cross-app
> `wandr:markdown/renderer.render`, displays the 11-block result. Logcat:
> ```
> standalone: preopened /data/.../assets → /assets (read-only)
> assets: read("demo.md") → 1505 bytes
> [wasm] markdown-card: read assets/demo.md (1505 bytes)
> [wasm] markdown-card: render() → 11 blocks (full tree lifted)
> ```
> Edit the .md file + reinstall = different content rendered, no rebuild
> needed of any wasm.

## Why this matters

The current wandrpkg layout is:

```
wandrpkg/
  package.toml
  components/<name>.wasm
```

No place for `.md` files, images, fonts, .json configs, anything other
than wasm components. In task 36's `MarkdownCard` proof, the markdown
source ended up as a `private const val SOURCE = """ ... """` inside
the Kotlin guest — works for a demo, doesn't fit any real app pattern.

A typical Compose app needs:
- Bundled data files (markdown, JSON, SVG, glTF, …)
- Bitmap images (PNG/JPG/WebP)
- Custom fonts (TTF/OTF)
- Optional shaders, audio clips, localization tables, …

All of these are static content shipped with the app, read at runtime
by the guest. The installer needs to plumb them into the install dir
and the host needs to make them reachable from the guest.

## Two ways to plumb assets to the guest

| Approach | Guest reads via | Pros | Cons |
|---|---|---|---|
| **(A) WASI filesystem preopen** | `wasi:filesystem/types.open-at` + `read-via-stream` (Kotlin stdlib wraps this) | Standard; already in linker (`add_to_linker_sync`); guest code uses idiomatic Path/File APIs; supports streaming for large files | Couples assets to a real filesystem path; harder to swap source (APK assets, network, …); KT-* wasi-filesystem APIs less battle-tested |
| **(B) Custom host WIT** (e.g. `wandr:assets/store.read(name) -> result<list<u8>>`) | `@WasmImport` call into host | Source-agnostic (disk today, APK / network later); smaller guest surface; aligns with host-driven model; no WASI fs subtleties | Yet another hand-written Kotlin binding; needs explicit list/exists/etc. verbs over time; rebuild host to extend |

**This task initially picked (A) but pivoted mid-implementation to (B).**
The pivot point: Kotlin/Wasm 2.4 stdlib turns out to ship NO filesystem
APIs (no `fd_read`/`path_open` wrappers, no `kotlin.io.path.Path`
implementation on wasi). Hand-writing 100+ LoC of preview1 file ops in
Kotlin to make a 30-line custom-WIT verb redundant didn't pencil out.

**Both wired in the end:**
- The WASI preopen at `/assets` is in place (`WasiCtxBuilder::preopened_dir`
  in standalone.rs + run_once.rs) — zero cost when unused, ready for a
  future Kotlin stdlib that grows file APIs.
- The custom `my:skiko-gfx/assets.read` WIT verb is what guest code
  actually calls today (`wandr-app/src/wasmWasiMain/kotlin/AssetsImports.kt`).

## Manifest schema extension

`package.toml` gains an optional `assets` key — a directory path
relative to the wandrpkg root:

```toml
[package]
app_id      = "com.example.wandr-app"
version     = "0.0.1"
world       = "wandr:app/wandr-app"
composition = "same-store"
assets      = "assets/"   # ← new; optional; relative to wandrpkg root

[components]
ui = "components/ui.wasm"
```

If present and the dir exists in the bundle, the installer recursively
copies it into `<install_dir>/assets/`. If declared but missing →
install rejects with a clear error. If absent → no assets dir at all
(backwards-compatible; existing fixtures unaffected).

## Installer changes

`wandr-host/src/app_installer.rs`:
1. `parse_manifest` reads `assets` (optional `String`).
2. Manifest field carries through to install pipeline.
3. After component copy, if `assets` is set: `fs::create_dir_all` +
   recursive copy from `<bundle>/<assets>` to `<install_dir>/assets/`.
4. **Cache key:** assets are not AOT-compiled, so they don't gate the
   cwasm cache. But the install_dir's `assets/` IS the source of truth
   at runtime — if it drifts from what we shipped, the guest sees the
   newer file (intended). Skip hash tracking for now; revisit if asset
   hot-reload races become a problem.

## Host runtime wiring

Both `standalone.rs` and `run_once.rs` build their `WasiCtxBuilder`.
Today: `inherit_stdin/stdout` + `LogcatStderr`. Add:

```rust
if let Some(assets_dir) = loaded.assets_dir() {
    wasi_builder.preopened_dir(
        assets_dir,           // host path
        "/assets",            // guest mount point
        DirPerms::READ,       // read-only
        FilePerms::READ,
    )?;
}
```

For this to work, `LoadedApp` needs to surface the install dir (or at
least the assets sub-path) for `AppRef::Installed` loads. Today it
holds `entry`, `source_label`, `engine`, `deps` — add an
`Option<PathBuf> install_dir` that `load_installed` populates and the
other variants leave `None`.

For dev paths (`DevCwasm`/`DevAsset`) there's no install dir → no
preopen → guest's `/assets` read fails with `ENOENT` (acceptable; dev
paths predate assets).

## Guest read pattern (Kotlin/Wasm)

Kotlin/Wasm + WASI provides basic file I/O through stdlib `kotlin.io.*`
on the wasmWasi target. First attempt:

```kotlin
val source = kotlin.io.path.Path("/assets/demo.md").readText()
```

If stdlib APIs trip over preopen semantics or character encoding,
fall back to hand-written WASI calls (`fd_read` after `path_open`)
similar to the canonical-ABI lifts in `MarkdownImports.kt`.

`MarkdownCard.kt` reads the file at composition time (inside the
existing `remember { ... }`), passes the resulting String to
`renderDocument(source)`. Error path: if read fails, the card shows
the IOException message instead of the rendered document.

## Implementation plan (5 steps — all landed)

| # | What | Files | Status |
|---|---|---|---|
| 1 | Scope doc | `tasks/38-wandrpkg-assets.md` | ✅ |
| 2 | Manifest + installer for assets | `wandr-host/src/app_installer.rs` | ✅ already wired pre-task (auto-detects `<bundle>/assets/` since task 35) |
| 3 | LoadedApp surfaces install_dir; WASI preopen + custom WIT impl in both entry points | `wandr-host/src/{app_loader,lib,standalone,run_once,assets_impl}.rs`; `wit/skiko-gfx.wit` adds `interface assets` + import to `skiko-ui` world | ✅ |
| 4 | Move SOURCE → `wandr-app/assets/demo.md`; MarkdownCard reads it via new hand-written Kotlin binding | `wandr-app/assets/demo.md` (new), `wandr-app/src/wasmWasiMain/kotlin/{AssetsImports,MarkdownCard}.kt`, `wandr-app/wit/deps/skiko-gfx/skiko-gfx.wit` (synced) | ✅ |
| 5 | Device verify | manual run + screenshot | ✅ 2026-05-26 |

## What's out of scope (this task)

- **Write access to a data dir** — apps that need to save state want a
  separate writable preopen at e.g. `/data`. Easy to add when first
  needed; same `preopened_dir` call with `DirPerms::ALL` + a writable
  install subdir like `<install_dir>/data/`.
- **Per-asset hashing in cache-key.toml** — assets don't gate cwasm
  reuse; add only if hot-reload semantics get muddy.
- **Custom host-WIT asset interface** (option B) — layer on later if
  alternative sources (APK assets, network) appear.
- **Asset compression / lazy loading** — premature; ship the simple
  preopen first.
- **Cross-app shared assets** — a separate `[asset_deps]` table that
  resolves shared bundles. Wait until two apps actually want to share
  the same asset bundle.

## Related

- `tasks/35-app-install.md` (single-app installer foundation).
- `tasks/36-cross-app-deps.md` (multi-package boundary; this is the
  "data" complement to that "code" boundary).
- `docs/architecture-host-guest-boundary.md` (the host-driven model;
  this task adds a host-driven file-read affordance).
- Memories: [[task-36-step-7-pending]], [[adb-push-dir-nesting-gotcha]]
  (will bite asset dirs the same way it bit wandrpkg dirs).
