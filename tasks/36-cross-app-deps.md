# Task 36 — cross-app dependencies + system components

> **Status:** ✅ complete — steps 1–7 device-verified 2026-05-26.
> Full cross-app dep chain runtime-validated end-to-end: install both
> packages → resolver → loader → `wire_markdown_dep` proxy →
> `Command::instantiate` → `wasi:cli/run.run` → consumer calls
> `render()` through proxy → dep returns Document → consumer exits 0.
>
> **Step 7 deliverables (this session, 2026-05-26):**
> - `wart-host/src/run_once.rs` — new entry point that drives one-shot
>   `wasi:cli/command` consumers via `Command::instantiate` +
>   `call_run`. Sets up the same SF surface + HostState + WASI ctx as
>   `standalone.rs` (renderer-Option refactor deferred — see below).
> - `LoadedApp::instantiate_command` in `app_loader.rs` — parallel to
>   `instantiate`, swaps `SkikoUi::instantiate` for
>   `wasmtime_wasi::p2::bindings::sync::Command::instantiate`.
>   Crucially, `wire_dep_into_linker` runs identically — the proxy
>   registration is consumer-shape-agnostic.
> - `wart-host --run-once <app-id>` CLI in `main.rs`, adjacent to
>   `--install` / `--standalone`.
> - `md-smoke-rust/` — new Rust-side smoke consumer. Built because the
>   Kotlin smoke (`wart-app-md-smoke/`) hits a pre-existing Kotlin/Wasm
>   + WASI-command-adapter throw at module init (confirmed on-device
>   2026-05-26 — wart-host's `wasi_stderr` doesn't help). Rust on
>   `wasm32-wasip2` produces a clean wasi:cli/command shape with no
>   such bug.
> - `scripts/smoke-markdown.sh` — full device pipeline (build → install
>   both packages → `--run-once` → grep logcat).
> - `docs/architecture-host-guest-boundary.md` — captures the
>   host-driven cardinality-1 framing (one-shot CLI is the same
>   primitive as `renderFrame`, just N=1 instead of N=60×/sec).
>
> **Device evidence (2026-05-26):**
> ```
> loader: cache fresh for com.example.md-smoke-rust 0.0.1
> loader: loaded dep `markdown` (war:markdown/renderer@0.1.0) from
>   .../system-apps/war.markdown.renderer/0.1.0/cache/renderer.cwasm
> loader: dep `markdown` instantiated; wiring war:markdown/renderer@0.1.0
>   → consumer linker
> run_once: command instantiated — calling wasi:cli/run.run()
> md-smoke-rust: render() returned [N block(s)]
> run_once: call_run returned Ok — guest exited cleanly
> ```
> EXIT=0. The Kotlin smoke separately validates the install/load/linker
> layer up to `Command::instantiate` (same logs minus the last two);
> the Rust smoke proves the actual call-through-proxy path.
>
> **Compose consumer also verified (visuals v1, 2026-05-26):** wart-app
> installed via the installer path (new `wart-app/wit/` defines a world
> `wart:app/wart-app` that `include`s `my:skiko-gfx/skiko-ui` and
> imports `war:markdown/renderer@0.1.0`), new `MarkdownCard.kt` /
> `MarkdownImports.kt` call `render()` once at composition time.
> `wart-host --standalone --app com.example.wart-app` rendered:
> ```
> [wasm] markdown-card: render() → 5 blocks parsed by external component ✓
> ```
> Same `wire_markdown_dep` proxy as the CLI smoke; same install
> machinery; just a different consumer shape. Cross-app deps now proven
> for BOTH Compose (`SkikoUi::instantiate`) and CLI (`Command::instantiate`)
> paths.
>
> **What's deferred:**
> - **Rich document-tree rendering** in MarkdownCard — v1 only reads
>   `blocks.len`; v2 walks each `block` variant (paragraph/heading/
>   code-block/bullet-list/etc) + nested `run` styles, renders each as
>   a real Compose element. ~200-300 LoC of canonical-ABI lifting.
> - `HostState.renderer: Option<SkiaRenderer>` cleanup. `--run-once`
>   builds a real SF surface to avoid a ~222-site refactor across
>   `canvas_impl.rs` + `paragraph_impl.rs`. Costs a screen-flash for
>   the ~1s smoke. Revisit if more CLI shapes appear.
> - Kotlin/Wasm + WASI-command-adapter throw bug — scoped at
>   `tasks/37-kotlin-wasm-command-adapter-throw.md`. Module-init level,
>   unconditional; unrelated to dep wiring (Rust smoke + Compose smoke
>   both prove the wiring without needing this fix).
> - Separate-Store composition mode — markdown driver is same-Store;
>   wait for a service-shaped dep before building this.

## Why this matters

`tasks/35-app-install.md` handles one app — read manifest, precompile,
cache, deserialize, instantiate. That's strictly single-package.

In practice that's not enough:

- **App-to-app dependencies.** App A wants to use App B as a service
  (e.g. a "tenant database" app that 5 apps share; a "notes provider"
  another app's editor pulls from). B has to be installed first, A
  refuses if B is missing, B's update affects A.
- **System components.** Things like an emoji picker, a unit converter,
  a markdown renderer that the runtime ships with as part of the
  platform — pre-installed, trusted, available to every app as an
  import. Not part of the host binary (so they can be updated
  independently), but always present.
- **Capability provenance.** A's manifest says it imports
  `acme:notes/store`; that import has to be satisfied by *something*
  the installer can find. Without a graph, "I import X" can't be
  resolved at install time and only fails at instantiate time, which
  is the worst place for it.

This isn't a "later" question — the answer determines the on-disk
package layout and the installer's resolution algorithm. Getting it
wrong forces breaking changes to both.

## What wasmtime supports today (verified 2026-05-26)

Inspected `wasmtime-src/crates/wasmtime/src/runtime/component/linker.rs`:

| API | What it does | Status |
|---|---|---|
| `Linker::instance(name)` | Define a whole WIT interface in the linker by name | ✅ stable |
| `Linker::resource(...)` | Define a resource type the linker can produce/consume | ✅ stable |
| `Linker::substituted_component_type(c)` | Get a component's type with imports substituted from the linker | ✅ stable |
| `LinkerInstance::func_new` / `func_wrap` | Define host-provided implementations of WIT functions | ✅ stable |
| Instantiate ≥2 components into the **same** Store with one's exports wired to the other's imports | Multi-component composition at instantiation time | ✅ stable |
| **Lazy load**: load A first, later load B that satisfies A's missing imports without reinstantiating A | True dynamic linking | ❌ not stable (§7.4) |

Practical upshot: **you can do everything the user needs, today, at
install/load time** — not at runtime. The lazy "load on demand" the
user mentioned isn't there yet, but neither is it required for "A
depends on B; install B first, then A".

## Two composition modes (the real decision)

When A imports `acme:notes/store` and B exports it, where does B
*live* relative to A?

| Mode | A and B share | Wiring | Crash domain |
|---|---|---|---|
| **Same Store (library-like)** | One wasmtime `Store<HostState>`; one GC heap; one HostState | Linker composes B's exports → A's imports at instantiation time. Resource handles pass directly. | A and B die together. One's panic / OOM hits the other. |
| **Separate Store (service-like)** | Same wart-host process; separate `Store`s; separate GC heaps; separate HostStates | Host provides A a *proxy* implementation of B's WIT that marshals calls (and resources) across Stores via channels / `Mutex`. B has its own render loop / event loop. | Independent: B can panic without killing A. |
| **Separate process (Hybrid future)** | Different OS processes (Hybrid/zygote per §9). | Host provides a proxy that marshals over binder. | Full OS isolation; oom-killer can take one without the other. |

Concretely:

- A typical "shared library" dep — utility component, no state of its
  own, called synchronously — wants **same Store**. Cheap, low
  overhead, intuitive.
- A typical "service" dep — owns persistent state, may be slow,
  multiple apps connect concurrently — wants **separate Store** (or
  separate process post-Hybrid). Isolation matters.
- The component author can't decide alone; it's a system-level call
  (you can't let any app demand "give me same-Store with the camera
  service"). Likely: **the system component / package declares its
  composition mode**, and the installer enforces it.

## Two flavors of "system component"

The user's "system components that can be resolved runtime" splits
cleanly into two:

1. **Host-provided WIT (what we have today).** Implemented in the
   wart-host Rust binary; bound into every Linker via
   `add_to_linker_sync` / `SkikoUi::add_to_linker`. Examples:
   `my:skiko-gfx/*`, lifecycle, clipboard, scheduler, haptics, audio,
   sensors, power, thermal. Always available; updated with the host.
2. **Runtime-bundled components.** `.wasm` files shipped *with* the
   wart-host install — pre-installed under
   `/data/wart/system-apps/<id>/<v>/`. Same install layout as a
   regular app (§7.1b), but installed atomically with the runtime
   itself (not by a user). Useful for things that don't have to be
   native Rust — emoji-shaper, settings-store, notification-center,
   …. Updated as part of the runtime; user apps can declare a
   dependency on them.

These are the same to the *consumer*: both just appear as
`Linker::instance("acme:notes/store")` from A's point of view.
Different to the *installer*: host-WIT is always satisfiable;
runtime-bundled is checked against the system-apps registry; user-app
deps are checked against `/data/wart/apps/`.

## Manifest extension — `[dependencies]`

```toml
[package]
name    = "com.acme.editor"
version = "2.0.0"
entry   = "ui"
world   = "war:app/main@1.0.0"

[dependencies]
# A user app this installer must find under /data/wart/apps/. Refuse install
# if absent. Compose at install time (or first launch — see §Cache).
notes-store = { app = "com.acme.notes", version = "^1.0", interface = "acme:notes/store" }

# A runtime-bundled system component. Same lookup, different root:
# /data/wart/system-apps/.
emoji = { system = "war:emoji/shaper", version = "*", interface = "war:emoji/shaper" }

# A host-provided WIT. Optional to list — every world implicitly imports
# host WIT — but useful for explicit version pinning.
haptics = { host = "my:skiko-gfx/haptics", version = "1" }

[components]
ui = { path = "components/ui.wasm" }
```

The installer's resolver walks `[dependencies]`, resolves each to a
concrete `.wasm` (host-provided is a no-op; system-bundled looks up
`/data/wart/system-apps/`; app-typed looks up `/data/wart/apps/`),
and either generates a `link.wac` or composes at load time via the
Linker.

## Resolution algorithm (installer)

```
for each dep in package.dependencies:
    match dep.kind:
        host:   assert runtime offers WIT @ version; record in cache-key
        system: locate /data/wart/system-apps/<dep.system>/<resolved-version>/
                ensure installed; refuse if missing
        app:    locate /data/wart/apps/<dep.app>/<resolved-version>/
                ensure installed; refuse if missing
                determine composition mode (from dep's package.toml)

if any unresolved: refuse install with "missing dependency: X"

write link.wac:
  same-Store deps → wac instantiates dep into A's graph
  separate-Store deps → wac wires A's imports to host-proxy interface;
                        host generates the proxy stubs at load time

for each component in package:
    cwasm = engine.precompile_component(bytes)
    cache.write(cwasm)

cache-key includes hashes of all dep wasm bytes too
```

Uninstall: refuse to uninstall B if any A depends on it (or warn +
cascade — policy TBD). The reverse dep set is computable by walking
all `[dependencies]` lists in installed apps' manifests.

## Cache key extension (incremental over task 35)

`cache-key.toml` per install adds dep hashes:

```toml
wasmtime_version = "44.0.0"
engine_config_hash = "sha256:…"
[components]
ui = { wasm_sha256 = "…", cwasm_sha256 = "…" }
[dependencies_resolved]
notes-store = { kind = "app",    app = "com.acme.notes",      version = "1.4.2", wasm_sha256 = "…" }
emoji       = { kind = "system", id  = "war:emoji/shaper",    version = "0.9.1", wasm_sha256 = "…" }
haptics     = { kind = "host",   wit = "my:skiko-gfx/haptics", version = "1" }
```

Any dep update flips a hash → A's cache invalidates → re-precompile
on next launch (same mechanism as wasmtime upgrade). No special case.

## Where true lazy linking *would* have helped (and the workaround used instead)

Lazy linking — load A first, then later resolve A's missing imports
from B without reinstantiating A, OR hot-swap a dep while A is
running — is **not stable in wasmtime** as of 2026-05-26 (§7.4).
Audit of where this matters for the first cut, with the workaround
each uses instead:

| Case | Lazy would buy | Workaround in first-cut |
|---|---|---|
| **Cold-path / rarely-called dep** (A imports a video transcoder used in 5 % of sessions) | Don't precompile + instantiate B until first call | Treat as **separate-Store** dep. Host proxy holds an `OnceCell<Store<HostState>>` and only instantiates B on first call. Functionally lazy *at the Store level* even though the Component is precompiled at install. Stable today. |
| **Heavy / memory-constrained dep** | Defer the resident memory cost | Same — separate-Store + on-demand instantiation. |
| **Multi-version coexistence in one app** (A wants B@1 *and* B@2 simultaneously) | Both versions in one Store, resolved per call site | Two separate Stores, one per dep version; A talks to both via two host proxies. More boilerplate, works today. |
| **Plugin systems with no-relaunch UX** (host app picks up a newly-installed plugin while running) | Plugin instantiates on first use, live, no restart | **Relaunch consumer on dep install** — installer signals consumers; consumers next launch picks up the plugin. UX hit; works. Revisit if a concrete plugin host appears. |
| **Live service discovery** (A is a notification framework; new apps register a `notification-source` interface at install) | A sees new sources without restart | **Registry-component pattern** — system-bundled "registry" component all sources push to; A reads from it. State lives in the registry, not in lazy instantiation. Sidesteps the lazy requirement entirely. |
| **OTA component update without restart** | New dep version picked up by running consumer | Restart on host/dep upgrade (standard for any platform). |

**Marker for revisit:** when wasmtime's lazy-component-instantiation
APIs stabilize (track via wasmtime release notes / §7.4 re-check),
the first two rows (separate-Store + OnceCell) can collapse into a
genuine same-Store lazy import, dropping the host-proxy boilerplate.
The plugin and live-discovery rows would also become cleaner.

Until then: separate-Store mode is the lazy-emulation knob, and the
workaround table above is what each case maps to. The scope does
**not** block on lazy linking; first-cut implementation should
proceed without it.

## What stays out of scope (for the first cut)

- **True lazy linking.** Not stable in wasmtime; not needed for
  "install B then A" (workarounds tabled above).
- **Capability gating per dep edge.** Q5 said `link.wac` is the
  authority; per-dep finer-grained denial (e.g. "A depends on B but
  only A.ui can use it") is a `link.wac` authoring concern, not new
  infrastructure.
- **Service discovery / hot-pluggable services.** All resolution is
  at install time; no "find me any provider of WIT X at runtime".
- **Distributed deps** (a dep on another device). Not now.
- **Generic dependency-solver / SAT.** Use the cargo-lite approach:
  each dep names an exact app-id + a semver range; resolver picks the
  highest installed match or refuses. No backtracking, no version
  unification across the graph (each install just sees what's
  currently on disk).
- **Per-launch composition for service-like deps.** First cut: resolve
  at install; cache is invalidated on dep update. Per-launch
  resolution belongs with future true-dynamic-linking work.

## How this layers on `tasks/35-app-install.md`

Task 35 (single-app install) shipped 2026-05-26 — `WartInstaller` +
`WartLoader` + on-device AOT cache + drift self-heal are all live and
device-verified. Task 36 extends that surface incrementally:

1. **Task 35** (done): installer + loader, single-app, no deps.
   `[dependencies]` table ignored if present.
2. **Task 36** (this doc): dependency resolver in the installer;
   system-apps registry; Linker-time composition for same-Store deps;
   host-proxy generator for separate-Store deps; cache-key extension.

System-component bundling (runtime-bundled `.wasm`s shipped with
wart-host) is a separable third task — needs build-time tooling to
package them into wart-host's installed asset set. Out of scope of
this doc.

## Composition strategy — Q6 resolved 2026-05-26

**Decision:** explicit-required declaration. The producer's
`package.toml` MUST contain `composition = "same-store"` or
`composition = "separate-store"` in the `[package]` table; the
installer rejects packages that omit it. No default — every component
author is forced to think. Consumers cannot override.

```toml
# producer (markdown-renderer's package.toml):
[package]
name        = "war:markdown/renderer"
version     = "0.1.0"
world       = "war:markdown/renderer-world"
composition = "same-store"   # required; "same-store" | "separate-store"
```

**Driver for this call:** the markdown renderer (chosen 2026-05-26 as
the concrete second component — library-like, naturally same-Store).
Was at risk of being decided in the abstract; landing the rule
alongside the first real use sidesteps that.

## Implementation plan — markdown-renderer driver

Now that Q6 is settled and the driver is the markdown renderer, the
work breaks into seven steps. End-to-end goal: a tiny smoke consumer
imports `war:markdown/renderer` and calls its `render` export at
startup; output reaches WASI stderr via logcat.

| # | What | New files / surface |
|---|---|---|
| 1 | **WIT contract** — `wit/markdown.wit` defines `world renderer-world` with one export interface `markdown { render: func(source: string) -> document; ... }`. `document = record { blocks: list<block> }`, `block = variant { paragraph(list<inline>), heading(heading-block), code-block(code-block), list(...), quote, rule }`. Compose-friendly shape (drops into LazyColumn). | `wit/markdown.wit` |
| 2 | **Renderer component** — Rust crate; `comrak` parser → WIT document; exports `markdown`. Builds to `markdown-renderer.wasm` (target `wasm32-wasip2`). Standalone-testable via `wasmtime run`. | `markdown-renderer/` (new Rust crate; sibling of `wart-host/`) |
| 3 | **Manifest schema extension** — extend `app_installer`'s `parse_manifest` to read `[package].composition` (required for system components) and `[dependencies]` table (three flavours: host / system / app). Validation: reject if producer omits `composition`; reject if a dep uses a flavour the installer can't resolve. | `wart-host/src/app_installer.rs` |
| 4 | **Installer resolver** — walk `[dependencies]`, for each dep locate `<root>/{system-apps,apps}/<id>/<resolved-version>/` (refuse install if missing), record dep's wasm sha256 + resolved-version in `cache-key.toml`'s new `[dependencies_resolved]` section. Add a `--root system\|user` CLI flag (or `kind = "system"` in package.toml) so the installer writes system bundles under `/data/wart/system-apps/`. | `wart-host/src/app_installer.rs`, `wart-host/src/main.rs` |
| 5 | **Loader composition** — `WartLoader::load_installed` reads `[dependencies_resolved]`, looks up each dep's cwasm, loads them all into the same `Store`, wires dep exports → consumer imports via `Linker::instance(name)`. Single composition mode for this iteration: **same-Store** (which is what markdown declares). | `wart-host/src/app_loader.rs` |
| 6 | **Smoke consumer** — `wart-app-md-smoke/`: tiny Kotlin/Wasm app that imports `markdown.render`, calls it once at startup with a hardcoded source, prints the document via WASI stderr. No Compose UI; just validates dep resolution + same-Store composition. | `wart-app-md-smoke/` (new; sibling of `wart-app/`) |
| 7 | **Device smoke** — install markdown system-bundle, install smoke consumer (which declares `dependencies.markdown = { system = "war:markdown/renderer", … }`), run `wart-host --standalone --app com.example.md-smoke`, observe logcat for "loaded installed:…" + the rendered document printed. | `scripts/smoke-markdown.sh` |

Out of scope this iteration (each could be a follow-up task):
- **Separate-Store mode + host-proxy generator** — wait until a
  service-shaped dep (e.g. notes store) actually exists.
- **System-component bundling** (shipping `markdown-renderer.wasm` *with*
  wart-host's APK / standalone install rather than via separate `--install`).
- **Multi-version coexistence** in one consumer (A wants B@1 AND B@2).
- **Reverse-dep tracking** for uninstall safety.

## First action for a fresh session

Pre-implementation gates closed 2026-05-26:
- Scope promoted to numbered task ✓
- Wasmtime Linker stability re-verified on 44.0.1: `Linker::instance` /
  `substituted_component_type` / `resource` / `instantiate` all stable;
  no `lazy.*instantiat` / `defer.*link` surface — §7.4 still accurate ✓
- Q6 settled — explicit-required `[package].composition` declaration ✓
- Driver chosen — markdown renderer (system-bundled, library-like,
  same-Store) ✓
- WIT return shape chosen — structured spans record ✓
- Smoke-consumer shape — tiny new `wart-app-md-smoke/` (WASI-stderr only,
  no Compose UI) ✓

**Next chunk:** step 1 in the implementation-plan table above — author
`wit/markdown.wit` (document/block/inline records + the
`renderer-world` export). Then step 2 — the Rust component crate.

Related: `tasks/35-app-install.md` (depended on; shipped 2026-05-26),
`post-art-roadmap.md` §7 (package shape) + §9 (Q5 link.wac authority,
Q5b open signing), memory `project-app-lifecycle-and-packaging`.
