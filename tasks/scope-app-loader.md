# Scope: app-loader — the multi-component package boundary

> Preparatory analysis, 2026-05-26. Carves the loader/lifecycle/
> window-manager boundary `post-art-roadmap.md` §7.6 has been calling
> for so today's hard-coded `Component::deserialize_file` path can be
> swapped for the multi-component package shape (§7.5) later without
> rewriting callers.
>
> The actual implementation will live in `tasks/35-app-loader.md` once
> this scope is accepted; this doc is the why + the contract.

## Why this matters now

`post-art-roadmap.md` §7.6 is explicit: **"Write `app-loader.rs` *now*
with this interface even though today's implementation only handles a
single `.cwasm`."** The cost of writing two call sites' worth of
loader code today is small; the cost of teaching every caller about
`package.toml` + `link.wac` + multi-component instantiation later is
not.

§9 just resolved (2026-05-26) to **monolithic-first, Hybrid (zygote)
production target.** The §9 rule:

> Keep the `app-loader.rs` and arbiter behind a boundary that does
> not bake in in-process assumptions, so the Hybrid (zygote-style)
> migration stays cheap.

Same boundary. So the app-loader is also the Hybrid-migration
boundary, not just the multi-component boundary.

Two concrete forcing functions either of which could land this year:

- **A second guest app.** Even a tiny second `.cwasm` (e.g. a settings
  panel) on the same device exercises everything multi-app: lifecycle
  arbitration, focus routing, the per-app HostState lifecycle, the
  AOT cache layout per package. Today's call sites assume one.
- **The Q5b signing decision.** When the package-signing format gets
  picked, verification belongs in the loader. Today there is no loader
  to put it in.

Neither is imminent. But the cost of the boundary now is bounded
(small Rust refactor, no behaviour change), and the cost of NOT having
it scales linearly with caller count once multi-app starts.

## Current state (what the loader has to absorb)

Two call sites today, both doing the same thing slightly differently:

| File | What it does |
|------|--------------|
| `wart-host/src/lib.rs:117–141` (NativeActivity / winit path) | Tries cwasm filesystem candidates (`/sdcard/Download/...`, app-external-files dir), falls back to APK asset `skiko-component.cwasm`. Calls `Component::deserialize_file` or `Component::deserialize(bytes)`. Calls `SkikoUi::instantiate(&store, &component, &linker)`. |
| `wart-host/src/standalone.rs:55–70` (post-ART standalone path) | Calls `Component::deserialize_file(&engine, "/data/local/tmp/skiko-component.cwasm")` directly; falls back to `run_test_loop` if absent. Calls `SkikoUi::instantiate`. |

Both also build `HostState` inline (renderer + scheduler + lifecycle +
wasi + table), build a `Linker`, run
`wasmtime_wasi::p2::add_to_linker_sync` + `SkikoUi::add_to_linker`.
That's the WIT-host wiring; same code, twice.

Engine is centralized: `App::make_engine()` in `lib.rs:87` already.
Standalone calls it too. That part is fine.

## End state (what the loader's caller will look like)

```rust
// In standalone.rs::run() and lib.rs::App::resumed():
let loader = app_loader::default_for_target();
let package_ref = app_loader::PackageRef::default_cwasm();   // today
let app = loader.load(&engine, package_ref)?;                // ← only this changes when packages land

let mut store = Store::new(&engine, host_state);             // unchanged
let bindings = app.instantiate(&mut store)?;                 // ← used to be SkikoUi::instantiate
```

Same shape on both call sites. The loader hides:
- *Where* the cwasm bytes come from (filesystem, asset, package dir)
- *Which* component is the entry (today: the only one; tomorrow: per
  `package.toml [package] entry = "..."`)
- *What* linker wiring the entry needs (today: just `SkikoUi`; tomorrow:
  per the component's WIT-import set)

## Trait + types (proposed)

```rust
/// A request to load. Today's variants:
///   * SingleCwasm — explicit cwasm path, or a list of candidates
///   * AssetCwasm  — bytes already in memory (APK asset case)
/// Tomorrow's variant:
///   * Package    — points at a `package.toml` (or its parent dir)
pub enum PackageRef<'a> {
    SingleCwasm { candidates: &'a [&'a Path] },
    AssetCwasm  { bytes: &'a [u8] },
    Package     { dir: &'a Path },              // stubbed in step 1
}

/// What the loader hands back. Owns the deserialized Component(s) and
/// knows how to instantiate the entry against a Store<HostState>.
pub struct LoadedApp {
    pub source_label: String,   // for logs ("cwasm:/data/local/tmp/...", "package:com.example.demo/1.2.0")
    entry: Component,
    linker: Linker<HostState>,
    // (future) extra_components: HashMap<String, Component>,
    // (future) link_script: ResolvedLinkGraph,
}

impl LoadedApp {
    pub fn instantiate(&self, store: &mut Store<HostState>) -> Result<bindings::SkikoUi>;
}

pub trait AppLoader {
    fn load(&self, engine: &Engine, r: PackageRef<'_>) -> Result<LoadedApp>;
}

/// `SingleCwasmLoader` — the today's implementation.
/// `PackageLoader`     — stub returning `Err(NotImplemented)` in step 1;
///                       filled in when Q5b/multi-component work begins.
```

Where this lives: **`wart-host/src/app_loader.rs`** (new file; §7.6
specified the name).

## What stays out of scope (explicit non-goals)

These would inflate the scope past "carve the boundary":

- **wac composition.** Don't parse `link.wac` yet. The Package variant
  returns `Err(NotImplemented)` until task 35.
- **`wkg` / Warg integration.** Don't pick a package transport yet —
  the §7.4 ecosystem call said wkg is pre-1.0 and not stable enough to
  commit to.
- **Signing.** Q5b is open; verification slot exists in the trait but
  every loader returns "unsigned" until the format is picked.
- **Hot-reload.** No watch-the-cwasm-mtime-and-reload.
- **AOT cache management.** Loader assumes the cwasm files exist;
  it does not AOT-compile. The
  `wasmtime compile --target aarch64-linux-android ...` pipeline
  (CLAUDE.md "Build pipeline") stays where it is.
- **Per-component capability gating beyond what `link.wac` would
  encode.** Q5 says link.wac is authoritative; today we have one
  component so this is moot.
- **HostState refactoring.** Loader takes `Engine` + returns
  `LoadedApp`; HostState is still built by the caller. (Doing both at
  once would couple the loader to renderer/scheduler/wasi
  lifetimes.) A follow-up may split out a `HostStateFactory` if the
  Hybrid `fork()` path needs it.

## Steps (when task 35 starts)

1. **Add `src/app_loader.rs`** with the trait + `LoadedApp` + `PackageRef`
   enum + `SingleCwasmLoader` + a `PackageLoader` stub.
   No callers touched yet.
2. **Refactor `standalone.rs`** to use it. Smallest blast radius
   first — the standalone path has a single explicit cwasm path.
   Verify on device.
3. **Refactor `lib.rs` (NativeActivity)** to use it. Move the
   cwasm-candidate-search into a helper that `SingleCwasmLoader`
   consumes via `PackageRef::SingleCwasm{ candidates: &[...] }`.
   Verify NativeActivity APK still boots + renders.
4. **Optional polish:** if HostState construction also moves into a
   helper, both call sites collapse to ~15 lines apiece — but this is
   nice-to-have, not required for the boundary to exist.

Total ~1–2 days. Mostly mechanical.

## Verification

- `bash scripts/standalone-launch.sh` brings up the standalone
  runtime with the demo cwasm; UI renders + types as before
  (`adb shell input keyevent KEYCODE_A` types into BasicTextField).
- `bash scripts/deploy.sh` (NativeActivity APK path) still boots.
- `git grep -nE "Component::deserialize|Component::from_file|SkikoUi::instantiate"`
  shows hits **only** inside `app_loader.rs` after the refactor.
- Adding a fake `PackageRef::Package` call site (in a #[cfg(test)]
  test) returns `Err(NotImplemented)` cleanly — proves the boundary
  is reachable for the future variant.

## First action for a fresh session

1. Read this scope doc and `post-art-roadmap.md` §7 + §9 (decisions
   resolved 2026-05-26).
2. Promote the scope into `tasks/35-app-loader.md` (CLAUDE.md
   numbered-task format) with the steps above as the implementation
   plan, status `🟡 in progress — step 1 starting`.
3. Add `wart-host/src/app_loader.rs` per the proposed trait + types.
   Register it in `wart-host/src/lib.rs` (`mod app_loader;`).
4. Refactor `standalone.rs` first (Step 2 above); device-verify
   before touching `lib.rs`.

Related: `post-art-roadmap.md` §7 (package shape) + §9 (runtime model
+ Hybrid migration); `tasks/33-boot-model-bringup.md` (standalone
call site that this refactors); memory
`project-app-lifecycle-and-packaging` (resolved decisions + Q5b open).
