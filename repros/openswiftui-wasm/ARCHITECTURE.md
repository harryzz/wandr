# OpenSwiftUI on wasm — Architecture & Dependency Map

> A ground-truth structural reference for the OpenSwiftUI stack and its wasm32-wasip1
> port. Built from the package manifests + real imports (not guesswork). Use this as
> the map to plan the port cleanly. Companion to `RESUME.md` (state) and the patches.
>
> **Attribution convention:** "upstream" = `OpenSwiftUIProject/*` + `jcmosc/Compute`.
> "fork" = `harryzz/*` (our wasm branches — see RESUME.md FORKS table). All WASI/wandr
> code lives in the **forks**; upstream has no wasm renderer.

---

## 0. The layered dependency model (the big picture)

Eight repos, four layers. Arrows = "depends on / built on".

```
┌── Layer 3: OpenSwiftUI package (the framework) ───────────────────────────┐
│   OpenSwiftUIExtension ─┐                                                  │
│   OpenSwiftUIBridge ─────┼─▶ OpenSwiftUI ──▶ OpenSwiftUICore ──▶ OpenSwiftUI_SPI │
│                          │        │              │  ▲                  │   │
│                          │        └─▶ COpenSwiftUI│  │ OpenSwiftUIMacros│   │
│                          │                        │  └──(build-time)───┘   │
└──────────────────────────┼────────────────────────┼──────────────────────┘
                            │ imports the *Shims products, never raw engines │
┌── Layer 2: Shims (backend selection — what OpenSwiftUICore actually imports)┐
│   OpenAttributeGraphShims   OpenRenderBoxShims   OpenCoreGraphicsShims      │
│           │                        │              OpenQuartzCoreShims       │
└───────────┼────────────────────────┼─────────────────────┼─────────────────┘
            │ (env selects backend)   │                     │
┌── Layer 1: Swift API over engines ──┼─────────────────────┼─────────────────┐
│   OpenAttributeGraph   OpenRenderBox │   OpenCoreGraphics   │  OpenObservation │
│        │                   │         │        │             │   OpenCombine    │
└────────┼───────────────────┼─────────┼────────┼─────────────┼──────────────────┘
         │                   │         │        │             │
┌── Layer 0: C/C++ engines ──┼─────────┼────────┼─────────────┼──────────────────┐
│   Compute (jcmosc) ◀────────┘  OpenRenderBoxCxx  (pure Swift) OpenObservationCxx │
│   = the AttributeGraph engine   └─▶ OpenCoreGraphicsShims                        │
└──────────────────────────────────────────────────────────────────────────────┘

Build-time only: OpenSwiftUIMacros, OpenObservationMacros ─▶ swift-syntax
Leaf libs: swift-numerics, swift-log, swift-crypto, SymbolLocator, Semaphore, swift-algorithms
```

**The one seam that matters for the wasm port:** the **Shims ↔ Cxx boundary** in
`OpenAttributeGraph`/`Compute` — Swift closures crossing into C++ via the `swiftcall`
ABI. That single boundary is where ~all the porting pain has been (see §4).

---

## 1. Repo inventory — where each component's deps live

| Repo | Local `/tmp` | Remote (upstream) | Fork branch (ours) | swift-tools | C++ |
|---|---|---|---|---|---|
| OpenSwiftUI | `OpenSwiftUI` | `OpenSwiftUIProject/OpenSwiftUI` @ `bb31b59` | `harryzz/OpenSwiftUI@wasm32-wasip1` | 6.x | via SPI/C |
| Compute | `Compute` | `jcmosc/Compute` @ `efb754b` | `harryzz/Compute@wasm32-wasip1-osp` | 6.2 | yes (engine) |
| OpenAttributeGraph | `oag-fork` | `OpenSwiftUIProject/OpenAttributeGraph` @ `f20328e` | `harryzz/OpenAttributeGraph@wasm32-wasip1` | 6.2 | yes |
| OpenCoreGraphics | `OpenCoreGraphics` | `OpenSwiftUIProject/OpenCoreGraphics` | (unmodified) | 6.1 | no (pure Swift) |
| OpenRenderBox | `OpenRenderBox-dep` | `OpenSwiftUIProject/OpenRenderBox` | (unmodified) | 6.2 | yes |
| OpenObservation | `OpenObservation-dep` | `OpenSwiftUIProject/OpenObservation` | (unmodified) | 6.1 | minimal (locking) |

**How OpenSwiftUI resolves these (`Package.swift` lines 884–942):**
- Local-deps mode (`OPENSWIFTUI_USE_LOCAL_DEPS=1`, what the wasm build uses): siblings
  at `../OpenCoreGraphics`, `../OpenAttributeGraph`, `../OpenRenderBox`, `../OpenObservation`.
- Default mode: the `OpenSwiftUIProject/*` URLs on branch `main`.
- `OpenCombine` (`from 0.15.0`), `swift-log`, `swift-crypto`: added only off-Darwin
  (`!buildForDarwinPlatform`). `swift-numerics`, `swift-syntax`, `SymbolLocator`: always.

---

## 2. The OpenSwiftUI package — internal target graph (Layer 3)

```
OpenSwiftUI_SPI        → OpenRenderBoxShims                  [C/ObjC/C++ bridge: headers,
   ▲                                                          private-fwk overlays, signpost]
   │
OpenSwiftUICore        → OpenSwiftUI_SPI, OpenSwiftUIMacros, [THE CORE — every subsystem]
   ▲                     OpenCoreGraphicsShims, OpenQuartzCoreShims,
   │                     OpenAttributeGraphShims, OpenRenderBoxShims, OpenObservation
   │
OpenSwiftUI            → OpenSwiftUICore, COpenSwiftUI,       [PUBLIC API + platform impls]
   ▲                     (+ the 4 Shims products)
   ├── OpenSwiftUIExtension → OpenSwiftUI                     [conveniences]
   └── OpenSwiftUIBridge    → OpenSwiftUI                     [bridge to native SwiftUI]

OpenSwiftUIMacros      → swift-syntax (SwiftSyntaxMacros, SwiftCompilerPlugin)   [build-time]
COpenSwiftUI           → (C/ObjC only; header search into OpenSwiftUI_SPI)
CGTK                   → systemLibrary(gtk4)   [only if renderGTKCondition — Linux]
SwiftCorelibs          → C header stubs for non-Darwin (Foundation/POSIX shims)
```

Layering, top to bottom: **Extension/Bridge → OpenSwiftUI (public) → OpenSwiftUICore
(everything) → OpenSwiftUI_SPI (C bridge)**. Macros are a separate build-time plugin
(this is why the wasm build's host-tool links must not get `-lwasi-*` — see RESUME 4b).

### OpenSwiftUICore subsystems (`Sources/OpenSwiftUICore/`)
The bulk of the framework. Largest-first, with what matters for rendering:

| Subsystem | Role | Port-critical files |
|---|---|---|
| **View/** (~136 files) | the view tree + declarative API | `View/Graph/ViewGraph.swift` (the reactive host, extends `GraphHost`); `View/View.swift` (`_makeView`, `@_typeEraser`); `View/TupleView.swift` (multi-child) |
| **Data/** (~68) | @State, bindings, environment, preferences, observation | `Data/Update.swift` (update dispatch ↔ AG invalidation); `Data/State/State.swift`; `Data/Combine/*` (off-Darwin fallback) |
| **Layout/** (~65) | geometry, alignment, stacks; AG-driven | `Layout/LayoutComputer/*` (where the layout engine reads cached attrs — the phase-3 wall lived here) |
| **Event/** (~58) | gestures, responders, hit-testing | exercised on wasm: pointer input arrives via `wasi:input-handlers/pointer-handler` → `on_pointer` SPI → `@State`, driving swipe-to-move (2048 is user-playable on device) |
| **Animation/** (~54) | transactions, timelines, springs | |
| **Render/** (~38) | **display-list construction + renderer vendors** | `Render/DisplayList/DisplayList.swift` (the model); `RendererConfiguration.swift` (renderer selection); the vendors below |
| **Graphic/** (~32) | Color, Gradient, BlurStyle, Appearance | `Color.Resolved` (sRGB → the wandr sink reads these) |
| **Shape/** (~35) | Path + shape primitives | |
| **Util/** (~37) | threading/dispatch/runloop | `WasmThreadingShim.swift`, `WasmDispatchShim.swift`, `RunLoopUtils.swift`, `TimerUtils.swift` (the WASI substrate) |
| **Runtime/** (~3) | type ↔ AG registration | `Runtime/TypeConformance.swift` (`swift_conformsToProtocol` C-shim lives here) |

**Render/DisplayList — the renderer vendors** (`Render/DisplayList/`):
- `DisplayList.swift` — the resolved model: `Item{frame, value}`, `value = .content(Content) | .effect(Effect, DisplayList) | .states | .empty`; `Content.Value = color|shape|text|image|flattened|…`; `Effect = opacity|clip|transform|mask|…`.
- `DisplayListViewRenderer.swift` — vends the concrete renderer per `RendererConfiguration` (`.default`/`.rasterized`/`.stdout`/**`.wandr`**).
- `CAHostingLayer.swift` — Darwin (CoreAnimation). `StdoutRendererHost.swift` + `DisplayListStdoutRenderer.swift` — debug text dump.
- **`WandrRendererHost.swift` + `WandrDisplayListRenderer.swift`** (ours) — walk the DisplayList → `WandrDrawSink` (→ wasi:canvas CGContext). `RendererConfiguration.swift` carries `.wandr(WandrOptions{surface, sink})` + the public `WandrDrawSink` protocol.

---

## 3. The Shims indirection (Layer 2 — how a backend is chosen)

OpenSwiftUICore imports **`*Shims` products, never the raw engine**. Each Shims target is
a thin adapter that re-exports + type-aliases one backend, selected by env at build time:

| Shims target | Backends | Selector |
|---|---|---|
| `OpenAttributeGraphShims` | native `OpenAttributeGraph` · Apple `AttributeGraph` · **`Compute`** · DanceUIGraph | `OPENATTRIBUTEGRAPH_OPENATTRIBUTESHIMS_COMPUTE=1` (what we use) → aliases `OAG*`→`AG*` (Compute) |
| `OpenRenderBoxShims` | Apple `RenderBox` · `OpenRenderBox` | `RENDERBOX` (Darwin default) |
| `OpenCoreGraphicsShims` / `OpenQuartzCoreShims` | Darwin `CoreGraphics`/`QuartzCore` · pure-Swift `OpenCoreGraphics` | platform |

Adapter files (e.g. `oag-fork/Sources/OpenAttributeGraphShims/Adapter/Compute.swift`) set a
`attributeGraphVendor` constant and re-export the chosen module. **For wasm we pin the
`Compute` backend** — so the real engine is `jcmosc/Compute` (our fork), reached via
`OpenAttributeGraphShims`.

---

## 4. ⚠️ THE coupling seam — Swift↔C++ `swiftcall` (where the port lives or dies)

This is the single most important thing for a fresh port. Both `Compute` and
`OpenAttributeGraph` expose a C++ engine to Swift; closures cross that boundary using the
**`swiftcall`** calling convention (an implicit context register), which **mislowers on
wasm** → `signature_mismatch` traps at `call_indirect`.

- Macro definitions: `Compute/Sources/ComputeCxx/include/ComputeCxx/AGBase.h` (`AG_SWIFT_CC(swift)` = `__attribute__((swiftcall))`, `AG_SWIFT_CONTEXT` = `__attribute__((swift_context))`); OAG mirror in `OAGSwiftSupport.h`.
- Closure storage: `Compute/.../Closure/ClosureFunction.h` (`Function = AG_SWIFT_CC(swift) Result(*)(Args…, Context AG_SWIFT_CONTEXT)`). **Note:** on wasm these macros are NOT empty — `ClosureFunctionCI` stays hard-swiftcall, which is why a plain-C `*C` variant can't just reuse it (see the phase-3 `cache_fetch` templatize).
- Swift side: `@_silgen_name("AG…")` declarations lower the call with the Swift CC.

**The fix pattern (per-function, the entire grind):** add a separate plain-C `*C` symbol —
C++ `extern "C" …C(…, fn_ptr, void* ctx)` + header decl `#if defined(__wasi__)` + Swift
`#if arch(wasm32)` routing through it with a non-capturing `@convention(c)` trampoline +
boxed/by-pointer context. Examples in the forks: `AGGraphInternAttributeTypeC` (synchronous),
`AGGraphSetUpdateCallbackC`/`SetInvalidationCallbackC` (stored, heap-boxed), and our
`AGGraphReadCachedAttributeC` (the multi-child wall).

**Still-unfixed bounded set** (trap as richer views reach them — same `*C` recipe, validate
in the 4s `repros/compute-wasm/computerun` harness first): `AGGraphSearch`, `AGTupleWithBuffer`,
`AGGraphWithUpdate`, `AGTypeApplyEnumData`/`MutableEnumData`, `AGSubgraphAddObserver`, `AGSubgraphApply`.

### 4a. Root cause (wasm spec + toolchain) — and whether it can be automated

**wasm spec.** `call_indirect` runtime-checks the callee's **structural** function type
against the call site's type immediate; mismatch ⇒ **trap** ("indirect call type
mismatch") — essentially a pointer-compare over `(params)→(results)`
([spec/instructions](https://webassembly.github.io/spec/core/syntax/instructions.html),
[function-references](https://github.com/WebAssembly/gc/blob/main/proposals/function-references/Overview.md)).
On wasm, arg count/types are part of a function's identity — there is no implicit register
to absorb a difference.

**swiftcall lowering.** On normal targets `swiftself`/`swifterror` ride in dedicated
registers; wasm has none, so the Swift wasm ABI makes them **explicit tail params on every
swiftcc function** — e.g. `func foo(_ value: Int)` → `(param i32 i32 i32)`
([WebAssembly/tool-conventions SwiftABI.md](https://github.com/WebAssembly/tool-conventions/blob/main/SwiftABI.md)).
So thin vs thick, non-throwing vs throwing, and **swiftcc vs plain-C** become *literally
different wasm types*.

**Two failure modes seen here.**
1. `call_indirect` type mismatch — a closure passed where a different arity/throws-ness is
   expected (the "wrong/undefined funcref" class).
2. **wasm-ld signature-mismatch stub** — when one symbol is declared with two signatures
   (swiftc `@_silgen_name` decl = 11 i32 incl. swiftself/swifterror + thick-closure words;
   clang `extern "C"` def = 9), wasm-ld emits a stub whose body is `unreachable` and points
   the bad call there ⇒ runtime trap. **This was `signature_mismatch:AGGraphReadCachedAttribute`.**
   (`--fatal-warnings` turns it into a link error instead.) ([lld WebAssembly](https://lld.llvm.org/WebAssembly.html)).

**wasmtime is the messenger, not the bug.** It only enforces the spec trap / runs the
`unreachable` stub; the mismatch is created at compile/link time (clang vs swiftc disagree
on the wasm type). Nothing to fix in wasmtime — its value here is observation
(`-D coredump`/DWARF named the symbol).

**No toolchain fix exists.** The auto-thunk that would convert thin↔thick / nothrow↔throws
at IR-gen ([swiftwasm/swift PR #6](https://github.com/swiftwasm/swift/pull/6)) was merged
into the swiftwasm fork (Dec 2019), **never upstreamed to `swiftlang/swift`**, then
**reverted** ("Revert thunk emission code completely"). Stock Swift 6.x does NOT auto-thunk;
a toolchain bump won't help. The per-function plain-C `*C` variant is therefore the
*sanctioned* workaround: it gives the symbol **one agreed C signature** both compilers emit
identically (no swiftcall, context as an ordinary `void*` + a `@convention(c)` trampoline).

**Can it be automated (a "macro to export for wasi")? Partly — and where matters:**
- **Swift has no C preprocessor** — no `#define FOO(x)` function-macros, only `#if os(WASI)`
  / `#if arch(wasm32)` conditional compilation. So you can't macro-generate this the C way in Swift.
- **Cheapest lever for NON-closure functions:** import with **`@_extern(c, "sym")` instead
  of `@_silgen_name("sym")`**. `@_extern(c)` binds with the **C** calling convention (no
  swiftself/swifterror), so plain engine functions stop mismatching with **no `*C` symbol at
  all** (the OAG fork already notes this in `Attribute.swift`). Only functions taking
  **swiftcc closures** still need a trampoline.
- **C++ side (where the `*C` symbols live): yes, a C-preprocessor macro fits** — the engine
  is already full of `AG_SWIFT_CC(...)` macros. One macro (e.g. `AG_C_FORWARD(name, ret, …)`)
  can expand, under `#if defined(__wasi__)`, to the `extern "C" …C(…, fn_ptr, void* ctx)`
  forwarder — removing the hand-copying.
- **Swift side: a Swift Macro (swift-syntax) can generate the boilerplate** — the
  non-capturing `@convention(c)` trampoline + the heap box + the `#if arch(wasm32)` routing —
  attached to the importing decl. Caveats: it **can't emit the C++ symbol**, and the shapes
  vary (synchronous vs stored callback, return type, retain semantics), so it's non-trivial.
  The project already ships swift-syntax macro plugins (OpenSwiftUIMacros / OpenObservationMacros),
  so the machinery exists.
- **Most pragmatic single mechanism:** a small **build-time codegen** that reads a manifest
  of the closure-taking engine exports and emits *both* the C++ `*C` wrappers and the Swift
  routing — one source of truth, no per-symbol drift. **Deepest fix:** make the engine's
  Swift↔C++ interface plain-C end-to-end, eliminating the whole class.

> Strategic read for a fresh start: the swiftcall mislowering is **systemic at this seam**,
> not a per-symbol accident. A from-scratch port should decide up front whether to keep
> grinding `*C` variants, or address the ABI at the boundary wholesale (e.g. a code-gen
> pass / a single C-ABI shim layer over the engine), and whether `Compute` is the right
> engine vs a thinner one. (See §7.)

Other platform divergence (much smaller): threading/dispatch/runloop in `Util/` (`#if os(WASI)`),
Text localization, Protobuf. `canImport(Darwin/UIKit/AppKit)` gates are pervasive (~313) but
are just "Darwin-only feature off" — low risk for wasm.

---

## 5. Status & maturity (upstream vs ours)

**Upstream (`OpenSwiftUIProject` + `jcmosc/Compute`):**
- Darwin (macOS/iOS): the real target — RenderBox (Apple private fwk) renderer, shipping.
- Linux (Ubuntu): builds + tests; **GTK4 backend is a stub** (no functional renderer); some features compiler-gated.
- **Android / Windows / WASI: not targeted upstream.**
- **Text rendering: not shipped** (Text + Text+Renderer are `Status: WIP`; off-Apple paths are `unimplemented` stubs).
- Compute = a reverse-engineering of Apple's AttributeGraph; functional, ~13 `fatalError`/"not implemented" stubs (debug/metadata, not core).
- Cross-platform OAG/RenderBox/CoreGraphics are "API-compatible, early dev."
- Source audit (OpenSwiftUI): ~568 files `Status: Complete`, ~96 `WIP`, plus TODO/Empty — concentrated WIP in Text, RenderBox/GraphicsContext, animation interpolation.

**Ours (the `harryzz` forks — all WASI/wandr work):**
- OpenSwiftUI compiles for wasm32-wasip1; renders a DisplayList incl. @State + multi-child VStack through the `.wandr` sink (desktop-verified). Text emits no glyphs yet (off-Apple stubs).
- Compute/OAG: WASI allocator/exceptions + the `*C` swiftcall variants.
- See `RESUME.md` for the exact branch/SHA per fork and the phase ledger.

**Known issue:** `OpenAttributeGraph#38` — WASI build hits a swift-wasm compiler internal
assertion (assertions-enabled toolchain). Largely the only wasm-tagged upstream issue;
WASI is otherwise undiscussed upstream.

---

## 6. Docs index

- In-repo: `OpenSwiftUI/Screenshots/Architecture/arch.png` (the renderer/DisplayList diagram); per-file `// Audited for <ver> / Status:` headers; repo READMEs (each states "open implementation of Apple's private framework X").
- Ours: `RESUME.md` (phase 1–4 narrative, build env, FORKS table), this file, `docs/swift-openswiftui-wandr-feasibility.md` (the original feasibility memo, phases 0–5), the 3 base-pinned patches.
- Harness: `repros/compute-wasm/` (the 4s Compute-ABI validation harness + its README on the `@_silgen_name`→C-import rule).

---

## 7. Implications for a clean-slate port (the actionable takeaways)

1. **The engine seam is the whole game.** ~all difficulty is the `swiftcall` Shims↔Cxx
   boundary (§4), not OpenSwiftUI itself. Decide the strategy *before* coding: keep the
   per-function `*C` grind, or do one systematic C-ABI shim / codegen over the engine.
2. **Pin `Compute` as the AG backend** via `OpenAttributeGraphShims` (env). Don't fight
   the other backends.
3. **Renderer = Option B (DisplayList walk → sink).** Don't try to make OpenSwiftUI's
   internal RenderBox/GraphicsContext path work on wasm; consume the resolved `DisplayList`
   and draw it yourself (`WandrDrawSink`). It's decoupled and already proven.
4. **Footprint is a hard constraint — and mostly fixable.** On-device AOT has a ~60–70MB
   cliff (debug 139MB OOMs). Biggest avoidable bloat: **swift-crypto is used for exactly ONE
   thing — `Insecure.SHA1` in `Data/Util/StrongHash.swift`** (the 160-bit internal identity/diff
   hash, gated `OPENSWIFTUI_SWIFT_CRYPTO`, one file) — yet it statically links the **entire
   BoringSSL** (the `CCryptoBoringSSL_*` AEAD symbols seen in the wasm). `StrongHash.swift` has
   only two SHA-1 backends (`#if OPENSWIFTUI_SWIFT_CRYPTO` swift-crypto / `#elseif canImport(CommonCrypto)`
   Darwin) — **no wasm fallback**, so you must add a third branch, then drop swift-crypto.
   Note "Swift+Foundation+BoringSSL" is three separate things: the Swift runtime+stdlib baseline
   (unavoidable short of Embedded Swift) and Foundation (large, only partly reducible) dominate;
   **BoringSSL is the smallest but most cleanly removable** — it alone may not clear the cliff
   (needs measuring). Two ways to remove it (both kill BoringSSL; no `wasi-crypto` needed):
   - **(A) Host-offload — the SHA-1 already exists.** `wandr:crypto`'s `interface hash`
     (`wit/crypto.wit`, linked into every guest via `CryptoHost::add_to_linker`,
     `app_loader.rs:284`) exports both a one-shot `digest(algo, data)` AND a streaming
     `resource hasher { create(algo)/update(data)/finish() }`. SHA-1 is a supported `hash-algo`;
     the streaming hasher maps 1:1 onto `StrongHasher` (create→update→finalize). Runs natively
     on the host (HW-capable per the CPU's `sha1` ext; the RustCrypto impl only uses the ARMv8
     SHA asm if built with that feature — currently only `aes_armv8`/`polyval_armv8` are set, so
     likely software-but-native). Reuses shipped task-93 infra; **but** couples OpenSwiftUICore
     to `wandr:crypto`.
   - **(B) in-guest header-only SHA-1 — ✅ CHOSEN.** Use `repros/compute-wasm/shims/openssl/sha.h`:
     a real ~35-line SHA-1 (zero deps) exposing the OpenSSL **streaming** API
     `SHA1_Init` / `SHA1_Update(ctx,data,n)` / `SHA1_Final(md,ctx)` + `SHA_CTX`, which maps **1:1**
     onto `StrongHasher` (`Insecure.SHA1()`→`SHA1_Init`, `.update(ptr)`→`SHA1_Update`,
     `.finalize()`→`SHA1_Final` → 5×UInt32). No wasm boundary crossing, fork stays wandr-agnostic.
     (Multi-buffer SIMD libs like mizchi/simd don't fit this access pattern — one small message at
     a time — and aren't wasm-SIMD as written.) **Integration:** (1) expose `sha.h` to
     OpenSwiftUICore on wasm via a tiny C module / the existing `-Xcc -isystem` shim path;
     (2) add the missing `#else` (wasm) branch in `StrongHash.swift` over `SHA_CTX`/`SHA1_*`;
     (3) drop swift-crypto (default-off off-Darwin) → eliminates BoringSSL.
     **✅ IMPLEMENTED + MEASURED (2026-06-19):** done as a pure-Swift `_OpenSwiftUIInsecureSHA1`
     in `StrongHash.swift` + `swiftCryptoCondition` default→false. BoringSSL gone (0 symbols,
     was 7691); builds clean. **BUT the stripped (DWARF+name) size dropped only ~1.3 MB
     (70.1 MB → 68.8 MB)** — BoringSSL was almost all DWARF/symbol-name weight; its linked code
     was ~1.3 MB (linker dead-strips the unused 99%). So **it does NOT clear the ~60 MB AOT cliff**
     — the bulk is Foundation + the Swift runtime/stdlib baseline + framework code (the real
     footprint target). (Gotcha hit: SPM caches the manifest by content-hash, so an env-only
     `OPENSWIFTUI_SWIFT_CRYPTO=0` does nothing — a Package.swift *content* change + `--manifest-cache
     none` + clearing resolved state is required to actually re-resolve.)
   - **Why (B) over (A):** `StrongHash` is hit per view-identity / display-list seed (likely hot),
     so (A)'s component-model call *per hash* would add boundary overhead; (B) has none. Switch to
     (A) only if profiling later shows SHA-1 is rare/large or you want the host HW path. (Audit
     `swift-log` similarly — off-Darwin-only.)
5. **Text IS part of this work (not a separate project).** Real apps (incl. swiftui-2048) need
   glyphs — when Text is used in the framework it's ours to make work. It's `WIP`/stubbed
   upstream off-Apple, so the port must implement it: the sink's text path AND filling
   OpenSwiftUI's off-Apple Text layout stubs (`Text+View.swift` sizeThatFits/spacing/
   explicitAlignment, `ResolvedText`, `ShapeStyleRendering`).
6. **Validate ABI shapes in the 4s harness, never the 90s probe first** (`repros/compute-wasm/computerun`).
