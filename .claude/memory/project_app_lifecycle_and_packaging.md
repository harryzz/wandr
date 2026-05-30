---
name: project-app-lifecycle-and-packaging
description: "Two architectural pillars from post-art-roadmap.md §7 + §9 — how WASM apps map to wasmtime instances, and how apps are packaged/installed"
metadata: 
  node_type: memory
  type: project
  originSessionId: be47cfff-188f-4f12-989d-c09046736d6a
---

The two foundational post-ART questions: (1) one wasmtime per app vs
one runtime hosting many vs hybrid; (2) what an "app" actually IS on
disk (manifest, components, assets, AOT cache). Both are scoped in
`post-art-roadmap.md` §7 + §9. **§9 was fully resolved 2026-05-26**
— see "Decisions" below.

**TL;DR:** monolithic now / Hybrid (zygote-style) for production;
multi-component packages with `package.toml` + `link.wac`; SF + InputFlinger
kept; APK ecosystem out of scope; per-component caps via `link.wac`.
One question still open: package signing format (Q5b).

## (1) Runtime model — monolithic / process-per-app / hybrid

Three shapes analyzed in roadmap §9:

| Shape | What it is |
|---|---|
| **Monolithic** | One WAR process; one `wasmtime::Engine`; many `Component` instances; in-process arbiter |
| **Process-per-app** | One WAR process per app; tiny session-manager owns SF/input/audio policy; binder arbitration |
| **Hybrid (zygote)** | Preload `wasmtime::Engine` + skia + AOT + font caches **once**, `fork()` per app, COW-share read-only pages |

**Current state (2026-05-26):** monolithic, single app, PoC working
end-to-end. Boot-model bring-up (task 33) all landed monolithic.

**Decisions (locked 2026-05-26):**
- **Monolithic = DECIDED** for PoC + boot-model bring-up. Reason: it's
  where the runtime is; a 2nd wasmtime+skia+AOT ≈ 95 MB cwasm + 50 MB
  host code/data duplicated, well past viability thresholds for
  process-per-app; in-process arbiter is trivial.
- **Hybrid (zygote-style) = production target.** Mirrors Android's
  actual model (zygote forks COW-sharing the preloaded ART runtime).
  Recovers the three-tier failure domain (app crash / framework crash
  / kernel crash) that monolithic loses. **Trigger to actually build:
  ≥2 concrete apps with a real user AND wasmtime DRC auto-scheduling
  fixed upstream** (otherwise even Hybrid can't isolate one app's GC
  stall from itself).
- **Process-per-app = REJECTED** on the memory math alone — no need to
  benchmark.

**Hybrid empirical update (2026-05-27, task 45 spike — commits
ad82c11/353f690/1c5a6927/462d53a5):** spiked a working
wart-zygote (native Rust, preloads `wasmtime::Engine`, fork()s
on UNIX socket LAUNCH commands, children run either
`wasi:cli/command` or full Compose render loop). 5 steps device-
verified end-to-end. The technical path is real, no fork-time
landmines on this stack (see [[wart-zygote-fork-survival]] for
the empirical findings). **But the memory math came in lower
than the scope-doc target**: ~5.6 MB Shared_Dirty per child, not
the ≥30 MB target. Per-app working set ~180 MB dominates the
COW savings. The Hybrid win is therefore **about three-tier
isolation, not memory** — same lesson stock Android learned.
This doesn't change the §9 lock; it sharpens it. Production
work (preload registry to grow COW savings to ~25-40 MB, real
wart-arbiter, SIGCHLD reap, init.rc/sepolicy) spun out as
recommended task 46 in `tasks/45-wart-zygote-spike.md`.

**Caveat — monolithic is a single point of failure:** a host crash
takes down every app + arbiter. Strictly worse isolation than stock
Android. Hybrid recovers it.

**Caveat — wasmtime DRC GC issue compounds multi-app:** see
[[wasmtime-drc-no-autoschedule]]. One app's GC stall freezes all in a
monolithic process. Multi-app under monolithic should not ship until
DRC auto-scheduling is fixed upstream OR we move to Hybrid (which
isolates GC stalls per process).

**Note on the old "decision blocker":** the roadmap originally framed
this as "measure cold-start of 2nd wasmtime; if <500 ms + <20 MB/app,
process-per-app is viable." Resolved 2026-05-26 by inspection: the
cwasm size alone (~95 MB) disqualifies process-per-app — no
measurement needed.

**Forward-compatibility rule:** even though monolithic ships now, keep
the app-loader and arbiter behind a boundary that doesn't bake in
in-process assumptions. Hybrid migration must stay cheap (the
`fork()`-shared wasmtime engine is API-compatible; what changes is how
`HostState`s and arbiter messages cross the process boundary).

**`fork()` constraint for the eventual Hybrid:** must fork *before*
wasmtime worker threads or EGL/GPU init. Android's zygote forks before
starting most threads for exactly this reason.

## (2) Package shape — what an "app" is on disk

Roadmap §7 (revised 2026-05-26). **Ship `.wasm`, cache `.cwasm`** —
direct parallel to Android's APK → dex2oat → `/data/dalvik-cache/`.

Shipped artifact (`.warpkg`, same bytes for every device):

```
<app-id>-<version>.warpkg/
  package.toml           ← metadata + entry + declared world
  link.wac               ← composition script
  components/
    ui.wasm   logic.wasm   persist.wasm
  assets/fonts/, images/
  SIGN                   ← signature(s), format Q5b-pending
```

On-device cache (per install dir, regeneratable):

```
/data/wart/apps/<app-id>/<version>/
  package.toml + link.wac + components/ + assets/   (copies of shipped)
  cache/
    ui.cwasm   logic.cwasm   persist.cwasm         (Engine::precompile_component output)
    cache-key.toml                                  (wasmtime-ver, config-hash, bytes-hash)
```

`wasmtime::Engine::precompile_component(bytes) -> Result<Vec<u8>>` is
stable in wasmtime 44 (already pinned) — the one-call API for on-device
AOT. Same Cranelift path the `wasmtime compile` CLI uses on the dev
machine.

Minimum manifest (§7.5, subject to revision):

```toml
[package]
name    = "com.example.demo"
version = "1.2.0"
entry   = "ui"
world   = "war:app/main@1.0.0"

[components]
ui     = { path = "components/ui.wasm",     aot = "ui.cwasm" }
logic  = { path = "components/logic.wasm",  aot = "logic.cwasm" }

[link]
script = "link.wac"

[assets]
dir    = "assets"
```

**Two-module split** (revised 2026-05-26 — see `tasks/35-app-install.md`):

- **Installer** (`wart-host/src/app_installer.rs`, new): replaces
  `PackageManagerService` + `dex2oat`. Reads `package.toml`, verifies
  world + signature, copies `.wasm`s + assets to install dir,
  calls `Engine::precompile_component` per component, writes cache +
  `cache-key.toml`, registers in package db.
- **Loader** (`wart-host/src/app_loader.rs`, new — thin): looks up
  app-id → re-verifies cache-key (re-precompile if stale) →
  `Component::deserialize_file` → returns `LoadedApp`. Caller
  unchanged: still `Store::new` + `app.instantiate(&mut store)`.

The dev workflow (CLAUDE.md "Build pipeline" → adb push to
`/data/local/tmp/skiko-component.cwasm`) bypasses the installer
entirely. Both call sites today (NativeActivity `lib.rs`, standalone
`standalone.rs`) inline `Component::deserialize_file` →
`SkikoUi::instantiate`; both get refactored behind the loader trait.

**Why nicer than APK semantics** (§7.3):
- **Permissions = WIT imports.** A component's imports literally *are*
  its capability requests; runtime grants/refuses by
  providing/withholding the host impl. No `<uses-permission>` XML drift.
- **Updates per-component.** Re-AOT 5 MB instead of 60 MB. Impossible
  with APK / DEX.
- **Linking declarative.** Auditable separately; signing the link
  decisions is meaningful (vs APK whose behaviour signatures can't
  summarise).

**Ecosystem state (Jan 2026 snapshot — re-verify before relying on
recent changes):**
- `wac` (composition language) — usable.
- `wkg` / Warg (registry + OCI transport) — pre-1.0; verify current
  version before committing to it as the on-device package transport.
- True runtime dynamic linking (lazy load on demand) — **not stable in
  wasmtime.** What IS stable: build-time-style composition done at
  load time, which covers "install an app, run it."
- Cross-component resource/handle delegation — works, rough edges.

**Standing rule (§7.6):** write `app-loader.rs` *now* with this
interface even though today's implementation only handles a single
`.cwasm`. Don't bake one-cwasm-per-app into the loader contract;
loader / lifecycle / window-manager boundaries must stay
forward-compatible with multi-component packages.

## §9 resolutions (locked in 2026-05-26)

- **Q1 — Runtime model:** ✅ monolithic-first DECIDED for PoC +
  boot-model; process-per-app REJECTED on memory math (95 MB cwasm +
  50 MB host per app); Hybrid (zygote-style) is the production target,
  trigger = ≥2 concrete apps + DRC auto-scheduling fix landed
  upstream. Forward-compat rule: keep `app-loader.rs` + arbiter behind
  a boundary that doesn't bake in in-process assumptions.
- **Q2 — Display server:** ✅ keep SurfaceFlinger via `ISurfaceComposer`
  + libgui shim. KMS/DRM rejected (out of scope for Android-OEM phone
  target; SF is already C++/binder so doesn't conflict with "remove
  Java" goal).
- **Q3 — Input source:** ✅ CLOSED — InputFlinger already shipped
  (task 33 Step 3 + hardware-key wiring). EVIOCGRAB rejected.
- **Q4 — Java-framework apps:** ✅ out of scope. No bytecode
  translation; new apps target Kotlin/WASM + Compose + WIT. Existing
  APK ecosystem intentionally not addressed.
- **Q5 — Per-component capability gating:** ✅ `link.wac` is the
  authority. Runtime grants caps by wiring host impls per script;
  refuses by leaving the import unwired (instantiate-time failure).
- **Q5b — Package signing format + trust roots:** 🔲 **OPEN** (newly
  carved out, was implicit under Q5). Format, trust-root distribution,
  revocation — all unspecified. Park until installable-package work
  begins.
- **Q6 — Cross-app composition default (same-Store vs separate-Store):**
  🔲 **OPEN** (added 2026-05-26 with `tasks/36-cross-app-deps.md`).
  Both modes stable in wasmtime today (verified via
  `wasmtime-src/.../component/linker.rs`); the call is which is the
  default and what the opt-out syntax is. Likely the dep's own
  `package.toml` declares mode authoritatively; consumer can't
  override. Defer until a concrete second component drives the call.

**Out-of-scope-of-§9-but-also-open:** on-device package location
(`/data/wart/apps/<pkg>/<version>/` plausible) + update/rollback
semantics. Pair with Q5b when installable packages get serious.

Related: [[wasmtime-drc-no-autoschedule]] (one app's GC stall would
freeze the whole monolithic process — argues for Hybrid before
multi-app); [[project-boot-model-libgui-build]] (the SF dependency
that resolving "display server" would have to walk away from); see
`post-art-roadmap.md` §7 + §9 for the full text and pro/con tables.
