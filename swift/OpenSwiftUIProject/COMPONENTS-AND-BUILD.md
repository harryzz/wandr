# OpenSwiftUI-on-wandr — components, forks, branches & build map

> Purpose: inspect **what is actually used, from where, on which branch**, how the
> build is wired, and the one real divergence (the wasi:canvas `CGContext`).
> Facts are read off `git`/`Package.swift`. Updated 2026-07-14 to reflect this session's
> cleanup: `print_cycle` moved into Compute (`ComputeStubs` deleted), the Apple shims moved
> to `swift/apple-compat/`, and the submodule `origin` remotes repointed to the harryzz forks.

---

## 1. Component inventory (actual on-disk remotes & branches)

All live under `swift/OpenSwiftUIProject/`. "Fork?" = did *we* (harryzz) modify it.

| Component | Remote (actual) | Branch | Commit | Ours? | What it provides |
|---|---|---|---|---|---|
| **OpenSwiftUI** | `harryzz/OpenSwiftUI` | `wasm32-wasip1` | `57d4c822` | **FORK** | The framework: OpenSwiftUICore (layout, DisplayList, **renderer + `WandrDrawSink`**), views, Button/gestures. |
| **OpenAttributeGraph** | `harryzz/OpenAttributeGraph` (`origin` ✅ repointed; `upstream` = OpenSwiftUIProject) | **`main`** | `6f2ab76` | **FORK (ours)** — 2 wasm commits over upstream | AttributeGraph **API shell** + shims → Compute. Newest wasm work is on `main`; the older `wasm32-wasip1` (`acf25d27`) is a superseded snapshot. |
| **Compute** | `harryzz/Compute` (`origin` ✅ repointed; `upstream` = jcmosc) | `wasm32-wasip1` | `332444d` | **FORK (ours)** | The **real graph engine** — the **two-weeks AttributeGraph fix; upstream Compute does NOT work.** Now also owns `Graph::print_cycle` (non-Apple no-op). Submodule `Submodules/swift-runtime-headers` → `jcmosc/swift-runtime-headers`. |
| **OpenCoreGraphics** | `harryzz/OpenCoreGraphics` @ `050239b` — **but `050239b` IS `OpenSwiftUIProject/OpenCoreGraphics` main, byte-identical** | detached @ `050239b` | `050239b` | **UPSTREAM, unmodified** (consumed). The harryzz repo only *parks* our wasi:canvas backend on a dormant side branch `wasm32-wasip1` (`00868a9`) that **nothing consumes**. | **GEOMETRY ONLY** as consumed: `CGRect`/`CGPoint`/`CGAffineTransform`/`CGPath`. `CGContext.swift` = empty 16-line stub (upstream). |
| **OpenObservation** | `OpenSwiftUIProject/OpenObservation` | `main` | `05e0581` | upstream, unmodified | `@Observable`. |
| **OpenRenderBox** | `OpenSwiftUIProject/OpenRenderBox` | `main` | `38d099f` | upstream, unmodified | ORB path storage / render primitives. |
| **OpenSFSymbols** | in-repo (`codeberg wandr`) | `openswiftui-eleev-2048` | — | **OURS** | SF Symbol name→glyph (bundled Tabler font). |

The app itself (eleev/swiftui-2048) is **not** here — it lives, UNMODIFIED, in
`repros/swift-canvas-spike/Sources/T2iles` and compiles behind the shims.

The **Apple-compatibility shims** (`SwiftUI`, `Combine`, `AudioToolbox`) now live in the shared
in-tree package **`swift/apple-compat/`** (not a submodule — wandr-original code), vending
Apple-named modules that forward to OpenSwiftUI / OpenCombine + fill gaps. Apps depend on its
products so unmodified `import SwiftUI/Combine/AudioToolbox` resolve. See §7.

---

## 2. ⚠️ The one real divergence — the wasi:canvas `CGContext`

`harryzz/OpenCoreGraphics` has **two branches**, and they carry different `CGContext`:

| Branch | Tip | `CGContext.swift` | Consumed? |
|---|---|---|---|
| `main` | **`050239b`** | empty 16-line stub (geometry only) | ✅ **the submodule is pinned here** |
| `wasm32-wasip1` | `00868a9` | **full wasi:canvas backend** (435 lines) + `WANDR_WASI_BACKEND.md` | ❌ **not checked out, not consumed** |

**What actually happens at build time:**

- OpenSwiftUI depends on `.package(path: "../OpenCoreGraphics")` → resolves to the
  pinned submodule = **`050239b` (geometry only)**. OpenSwiftUI needs *only* geometry,
  so the empty `CGContext` is fine for it.
- The **spike** needs the wasi:canvas `CGContext`, so it **VENDORS a copy**:
  `.target(name: "WandrCG", path: "repros/swift-canvas-spike/Sources/OpenCoreGraphics")`.
  It was renamed `WandrCG` to avoid a module-name collision with OpenSwiftUI's
  `OpenCoreGraphics@050239b` once both are in the graph.

So the wasi:canvas backend exists in **two copies that can drift**:

1. `harryzz/OpenCoreGraphics @ wasm32-wasip1` (`00868a9`) — the "official" home per its
   own `WANDR_WASI_BACKEND.md`, **but not consumed by any build**.
2. `repros/swift-canvas-spike/Sources/OpenCoreGraphics` (module `WandrCG`) — the copy
   that is **actually built**. It equals branch (1) **plus** local deltas: the
   `wandrDrawShapeCount`/`wandrDrawTextCount` verify counters, and (this session) the
   new `clip(svgPath:)` / `fill(svgPath:)` methods.

> **clip/fill (this session, landed):** the renderer half (`WandrDisplayListRenderer` +
> `WandrDrawSink`) is committed to `harryzz/OpenSwiftUI@wasm32-wasip1` (`54640751`). The
> `CGContext` `clip(svgPath:)`/`fill(svgPath:)` half is in the vendored `WandrCG` (committed in
> the wandr repo), **not** mirrored to OCG's dormant `wasm32-wasip1` — per §6 the CGContext
> backend lives in `WandrCG` until the `CWASICanvas` split. **GOTCHA fixed here:** `Path.forEach`
> is an unimplemented stub off-Apple (traps with `unreachable`); build SVG from `path.storage`
> / `roundedRect()` instead. Verified on desktop: oval pill buttons + rounded tiles.
>
> **Visual effects landed** (all renderer → WandrDrawSink → CGContext, framework half on
> `harryzz/OpenSwiftUI@wasm32-wasip1`, CGContext half in vendored `WandrCG`):
> - **clip / solid-shape fill** — oval buttons, rounded tiles. ✅
> - **3D tilt** — `rotation3DEffect` / projection via the canvas CTM (OpenSwiftUI computes the
>   matrix; we map ProjectionTransform → wasi:canvas 3×3, a transpose). ✅
> - **drop shadow** — `.filter(.shadow)` via the wrapped clip **silhouette** (blurred behind the
>   clipped card). ✅
>
> **POLISH TODO:** shadow *contrast* (the blur `radius → sigma` mapping is `sigma = radius`, likely
> needs tuning) and 3D-tilt *fidelity* vs the original. Still **dropped** (needs a WIT verb, not
> a renderer change): the frosted **backdrop blur** behind modals (`.filter(.blur)` — the
> wasi:canvas contract has no general layer/backdrop-blur verb, only per-paint `mask-blur`).

---

## 3. Build graph (what pulls what)

```
repros/swift-canvas-spike/Package.swift
├─ .package(path: swift/OpenSwiftUIProject/OpenSwiftUI)   [harryzz @ wasm32-wasip1]
│    ├─ ../OpenCoreGraphics     [050239b — GEOMETRY only, empty CGContext]
│    ├─ ../OpenAttributeGraph   [harryzz @ main, 6f2ab76] ──▶ ../Compute [harryzz @ wasm32-wasip1, 332444d]
│    ├─ ../OpenRenderBox        [upstream main]
│    ├─ ../OpenObservation      [upstream main]
│    ├─ ../OpenSFSymbols        [ours]
│    ├─ OpenCombine 0.15.0, SymbolLocator 0.2.0, swift-syntax 601, swift-numerics 1.0.3
│    └─ (Darwin only) DarwinPrivateFrameworks
├─ .package(path: swift/apple-compat)  ──▶ products: SwiftUI · Combine · AudioToolbox  [→ OpenSwiftUI/OpenCombine]
└─ targets:
     CSwiftSpike (wit-bindgen-c wasi:canvas/input surface)
     WandrCG  ◀── VENDORED OpenCoreGraphics wasi:canvas CGContext (the divergent copy §2)
     T2iles (@main = WandrReactor; eleev sources unmodified; excludes T2ilesApp/Audio/Plist)
     OpenSwiftUIDemo · SwiftSpike · ShimTest
```
(Removed from the spike this session: the `SwiftUI`/`Combine`/`AudioToolbox` shim targets — now
`swift/apple-compat` products — and `ComputeStubs` — `print_cycle` now lives in Compute.)

The app target (`T2iles`) per `Sources/T2iles/RULES.md` should carry ONLY **Audio /
Store / startup**; the `CGSink` + reactor currently in `WandrReactor.swift` are generic
runtime that belongs in the framework layer, not the app.

---

## 4. What was moved into `swift/` (history: session `05cfcba8`)

The user consolidated the tree ("lets move all OpenSwiftUIProject related repos in
`~/wandr/swift/OpenSwiftUIProject`"):

- **Repos → submodules** under `swift/OpenSwiftUIProject/` (the table in §1).
- **Probes/tests → `swift/OpenSwiftUIProject/tests/`**: `oag-baseline`, `asan-demo`,
  `vw-baseline`, `cpp-vec-probe`, the `cg*.gdb` scripts, `WASM-PORT-LOG.md`, `deploy`,
  `logs`, `RESUME.md`, `PR-description.md`.
- **wasi shims → `swift/OpenSwiftUIProject/wandr/wasi-shims/`**: `dispatch`, `openssl`,
  `syslog.h`, `wasi_compat.h`.

Explicit decisions from that session (verbatim intent):
- "consolidate branches first"; forks on `harryzz @ wasm32-wasip1`; OpenObservation /
  OpenRenderBox → **upstream** OpenSwiftUIProject (unmodified).
- **"#1, pin at `050239b`"** → OpenCoreGraphics pinned to `main`/baseline (geometry).
- "authorize push, register all 6 submodules"; "push wandr to codeberg".

---

## 5. ✅ RESOLVED — OpenAttributeGraph + Compute now use our `harryzz` forks

**Compute and OpenAttributeGraph are OUR forks. Upstream Compute does NOT work** — the fix
took ~two weeks. Both were previously checked out with `origin` pointing at UPSTREAM (with
`harryzz` only a *secondary* remote), so an in-tree `git submodule update` after a pin bump could
fall back to the non-working upstream. **Fixed this session** — remotes repointed + tracking set:

| Submodule | `origin` (now) | `upstream` (kept) | branch → tracks | pin |
|---|---|---|---|---|
| `Compute` | ✅ `harryzz/Compute` | `jcmosc/Compute` | `wasm32-wasip1` → `origin/wasm32-wasip1` | `332444d` |
| `OpenAttributeGraph` | ✅ `harryzz/OpenAttributeGraph` | `OpenSwiftUIProject/…` | `main` → `origin/main` | `6f2ab76` |

`.gitmodules` already named the harryzz URLs (so a fresh `clone --recurse` was always fine); the
repoint fixes the *existing* checkout's `.git/modules/*/config` and sets branch tracking. All of
this is **local submodule config — no parent-repo commit** (the pins are unchanged / already
committed). `OpenSwiftUI` and `OpenCoreGraphics` already had `origin` = `harryzz` (correct).

**OAG branch — canonical is `6f2ab76`** (2026-06-26, "wasm: Subgraph identity shim"). The two
harryzz OAG branches diverged; the newer + working one was on `main`, but the local checkout was
confusingly on a branch *named* `wasm32-wasip1` that pointed at *main's* commit. **Fixed**: the
local branch is now `main` @ `6f2ab76` tracking `origin/main`; the misleading `wasm32-wasip1` local
branch was deleted; the stale local `main` (`ac5885b`, an ancestor) fast-forwarded up — no loss.
`harryzz/wasm32-wasip1` = `acf25d27` (2026-06-19) stays a superseded snapshot, not pinned. Note
OAG's wasm work lives on `main` (unlike Compute/OpenSwiftUI on `wasm32-wasip1`) — that's the real
history; the fork branches were not rewritten.

---

## 6. The wasi:canvas `CGContext` home — CONSTRAINED by an already-made decision

> **Correction:** an earlier draft recommended moving the OCG pin to `wasm32-wasip1` and
> deleting `WandrCG`. **That is wrong and would break the build.** The consolidation commit
> `23c7d366` (2026-06-30) states it plainly:
>
> > "OpenCoreGraphics pinned at 050239b (upstream-buildable); its wasm32-wasip1 tip 00868a9
> > **imports CSwiftSpike and breaks the demo's OpenSwiftUI→../OpenCoreGraphics dep build**."

So the hard constraint: **OpenSwiftUI depends on `../OpenCoreGraphics` and must build without
CSwiftSpike.** Therefore:
- OCG-as-consumed-by-OpenSwiftUI **must stay geometry-only** (`050239b`). Do **not** repoint it.
- The wasi:canvas `CGContext` (which *needs* CSwiftSpike) **must live in a separate unit**.
  That unit is `WandrCG` today (vendored) — the vendoring is **deliberate**, not accidental.

Remaining (smaller) cleanup options — no build-breaking one among them:

**(b) Make the fork's `wasm32-wasip1` a *separate* backend package** (its own module name,
depends on OCG-geometry + CSwiftSpike), consumed by the spike instead of vendoring. One home,
no OpenSwiftUI-build impact. *Cleanest of the real options.*

**(c) Status quo (vendor `WandrCG`) + mirror.** Keep the vendored copy, but **every** backend
edit must also land on the fork's `wasm32-wasip1` branch or it is lost. This is exactly how the
current two-copy drift arose (e.g. this session's `clip/fill` went only into the vendored copy).

Until (b) is done: any `CGContext` backend edit must be applied to **both** the vendored
`WandrCG` **and** `harryzz/OpenCoreGraphics@wasm32-wasip1`, or the fork branch goes stale.

---

## 7. Target layering — how an app SHOULD depend (and how it does today)

### The rule

**An app names only the upper layer.** App source imports **`OpenSwiftUI` and nothing lower** —
exactly like an Apple app writes `import SwiftUI` and never reaches into CoreAnimation. The
wasi:canvas plumbing (`CSwiftSpike`, `WandrCG`, the reactor, the `@_cdecl` exports) lives *below*
the app and is pulled in for it, **never named or imported by app code**.

### Target state (bottom-up — everything depends DOWN)

```
CWASICanvas          leaf C module: the wasi:canvas bindings ONLY (draw/types/layout).
(depends on nothing)  A standalone package — NOT generated inside the app. [see §6, the CSwiftSpike split]
   ▲            ▲
   │            │
OCG CGContext   wandr-runtime            wandr-runtime = the "missing layer": the @_cdecl exports
target          (imports OpenSwiftUI      (on_frame/on_pointer/on_resize/next_frame_delay), the
   ▲             + CWASICanvas)           wasi:canvas embedding handshake, CGSink (WandrDrawSink→
   │                  ▲                   CGContext), frame pacing, and a `runWandrApp` runner
OpenSwiftUI ──────────┘                   (sibling to the framework's existing `runStdoutApp`).
(framework: App.main,
 AppGraph, WandrDrawSink,
 WandrRendererHost — NO CSwiftSpike)
   ▲
   │  (single upper-layer product)
  APP   →  source: `import OpenSwiftUI` only.  Package dep: one product (OpenSwiftUI / a wandr product).
```

- **OpenSwiftUI** owns the runner + rendering, all through the **abstract `WandrDrawSink`** — it
  imports **no** CSwiftSpike (see §6 for why it cannot).
- **wandr-runtime** is the only place the C-ABI boundary lives (exports + embedding + `CGSink`).
  It depends on OpenSwiftUI + the bindings; the app depends on **it** (or gets it transitively).
- **App** = `@main struct MyApp: App { … }` + views + one `dependencies:` product. Nothing else.

### The one mechanical caveat (don't confuse LINK with IMPORT)

On wandr the app executable **is** the final wasm component, so the runtime (`@_cdecl` exports,
reactor, `CSwiftSpike`, `WandrCG`) must still be **linked into** it. That is fine and matches Apple:
- ❌ app **enumerates + imports** the low-level targets — *today's state*.
- ✅ app depends on **one upper-layer product** that carries them **transitively**; they are linked
  but never *named* or *imported* by app code. `@_cdecl` exports survive linking from a dependency
  (they are the component's exports, force-kept). This satisfies the rule with zero compromise.

### How it is WIRED TODAY (partially cleaned up)

`repros/swift-canvas-spike/Package.swift`, target `T2iles`, after this session:

```swift
dependencies: [
    .product(name: "SwiftUI",      package: "apple-compat"),   // ✅ shared package, not app code
    .product(name: "Combine",      package: "apple-compat"),   // ✅
    .product(name: "AudioToolbox", package: "apple-compat"),   // ✅
    "CSwiftSpike", "WandrCG",      // ← STILL the violation: app names the wasi:canvas layer directly
    .product(name: "OpenSwiftUI", package: "OpenSwiftUI"),
]
```
`ComputeStubs` is gone (folded into Compute) and the Apple shims are out (→ `swift/apple-compat`).
What remains is `CSwiftSpike` + `WandrCG`: `Sources/T2iles/WandrReactor.swift` still `import`s both,
so the app still reaches **down** into the bindings. Closing that is the `CWASICanvas`/`wandr-runtime`
work below. Same smell as `Sources/T2iles/RULES.md` (an app should carry only Audio / Store / startup).

### Enabling moves — progress

- ✅ **Done this session:** Apple shims (`SwiftUI`/`Combine`/`AudioToolbox`) → `swift/apple-compat`;
  `ComputeStubs` → folded into Compute (`Graph::print_cycle`). The app no longer names those.
- ⬜ **`CSwiftSpike` → a standalone leaf `CWASICanvas`** (wasi:canvas bindings only; drop the
  input/export trampolines into the runtime). Removes the package-cycle that blocks anything above
  the app from importing the bindings (§6, and the package-vs-target cycle).
- ⬜ **A `wandr-runtime` product** (imports OpenSwiftUI + `CWASICanvas`) holding the reactor + exports
  + `CGSink`; add `runWandrApp` beside the framework's `runStdoutApp`.
- ⬜ **App collapses to** `dependencies: [<one wandr product>]`, source `import OpenSwiftUI`.
