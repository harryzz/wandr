# Flutter/Dart on wandr — feasibility memo (+ the Go UI question)

> Written 2026-06-11, last entry of the guest-UI survey (Slint shipped /
> Avalonia feasible / Qt no — see the sibling memos). Grounded in Flutter's
> documented web architecture + dart-lang/sdk upstream state.
> Status: **analysis only — blocked on toolchain, but the gate is actively
> CLOSING (deep dive 2026-06-13; re-examined 2026-06-18 — the blocker is "a WASI
> stdlib + one EH codegen flip," both bounded/upstream-owned/host-unshimmable, not
> a large effort; see that section below); architecture favorable.**

## Verdict

**Not practical today — but for the opposite reason from Qt, and of all the
"gated" frameworks this is the one whose gate is most concretely closing.**
Flutter's *architecture* is unusually well-shaped for wandr (its web engine
implements `dart:ui` in Dart over a Skia-command boundary — the skiko trick,
shipped by Google). The blocker is the *toolchain*: dart2wasm output still
needs a JavaScript host — the official docs (updated Dec 2025) state it
"doesn't support execution in standard Wasm runtimes like wasmtime." **But**
standalone (non-JS) support is in **active development**
([dart-lang/sdk#53884](https://github.com/dart-lang/sdk/issues/53884)) with
narrow, named blockers, an in-tree `dart2wasm_standalone_platform.dill`, and
the Dart team explicitly soliciting a **Rust/wasmtime integration** (which
is exactly wandr's host) — see the deep dive. So the 2026-06-11 verdict
("nonexistent toolchain, not our fight, don't") was too pessimistic: it's
**"wait, but watch closely — the gate is closing,"** not "don't." Component
model ([#56366](https://github.com/dart-lang/sdk/issues/56366)) is a
separate, untouched proposal but **not strictly required** (once standalone
modules run in wasmtime, the wandr path mirrors the Kotlin pipeline:
standalone module + hand-rolled WIT bindings + the component adapter).

## Why the architecture fits (better than any non-shipped option)

Flutter splits into the Dart framework and an engine that implements
`dart:ui` — and `dart:ui`'s Canvas IS SkCanvas, method for method
(drawRect/drawRRect/drawPath/drawImageRect/saveLayer/clipRRect/…). Three
properties line up with wandr:

1. **The web engine is the existence proof.** Flutter Web's `dart:ui` is
   written in Dart (`engine/lib/web_ui`) forwarding every canvas call over
   a boundary to CanvasKit — Skia compiled to wasm, driven via JS interop.
   "The framework runs in a sandbox, the Skia commands cross a boundary,
   Skia runs elsewhere" is *Flutter's own shipping web architecture*.
   Retargeting web_ui's CanvasKit bindings from JS-interop to
   `my:skiko-gfx` WIT imports is conceptually the same move we made for
   skiko/Compose — and the WIT's canvas surface covers dart:ui's (it was
   derived from skiko, which mirrors the same SkCanvas).
2. **Text is HOST-shaped — the Compose model, not the Slint model.**
   dart:ui exposes `Paragraph`/`ParagraphBuilder` (SkParagraph underneath);
   the framework never sees glyphs. That maps onto wandr's existing
   `paragraph` interface (which mirrors skparagraph via skiko) — no glyph
   verbs, no in-guest shaper, no font-bytes shipping. Binding-surface work,
   not architecture work.
3. **WasmGC is already paid for.** dart2wasm emits WasmGC — and wandr's
   wasmtime AOT flags already enable gc/function-references/exceptions for
   Kotlin. The device runtime could host a Dart WasmGC module *today* if
   one existed in WASI flavor.

## The blocker, precisely (rewritten after the 2026-06-13 deep dive)

dart2wasm's output imports its platform from **JavaScript**. Issue #53884
enumerates the dependency — it's a finite, tractable list, not a wall:
**event loop / async, printing, OS timers, weak maps & finalizers, stack
traces, double→String, regexes, math (sin/pow/exp…)**, plus strings. For
each, the Dart team's plan is (1) implement in Dart/intrinsically, (2) link
a wasm impl, or (3) import from the host. What this corrects from the
earlier draft: this is **funded, in-progress upstream work**, not a fork we
must heroically own.

Concretely, as of mid-2026:
- An in-tree **`dart2wasm_standalone_platform.dill`** (and a
  `dart2wasm_stringref_platform.dill`) already exist — a standalone platform
  is being built; it's just **not shipped in released SDKs yet**, and the
  docs still say browser-only. No public proof-of-life running in wasmtime
  yet, so: real progress, not done.
- **Named remaining blockers** for released standalone: (a) dart2wasm emits
  **legacy try/catch EH opcodes** that wasmtime rejects — it must migrate to
  the new `exnref` exception-handling (now in all browsers); (b) the
  standalone `.dill` isn't in releases. Both are bounded.
- **Strings:** dart2wasm uses the `wasm:js-string` builtins (externref to JS
  String); standalone needs either a runtime that provides js-string-builtins
  or the **stringref / in-wasm string** variant (`stringref_platform.dill`).
- Dart-team caveats (@simolus3): **stack traces and weak maps can't be fully
  supported** standalone — acceptable degradation for a UI guest.
- The team is **asking for community input on a Rust integration crate** —
  i.e. exactly a wandr-shaped host. Unusually aligned.

**Why this matters for wandr's runtime specifically:** the device host
already runs WasmGC with the new `exnref` EH + function-references (the AOT
flags enabled for Kotlin). So the moment dart2wasm's standalone path emits
WasmGC + `exnref` (not legacy EH), wandr's wasmtime is **already positioned
to load it** — the runtime side needs nothing new. And componentization
doesn't wait on dart2wasm growing native component support (#56366): the
Kotlin/C# `wasm-tools component new --adapt` route applies once a standalone
module exists.

The other integration shape — running the native C++ Flutter engine
host-side via the embedder API (the flutter-pi model) — is still rejected on
principle: a fat per-app native runtime with its own GPU context, bypassing
the wasm sandbox, the component model, and `wasi:canvas`. That's "Flutter
beside wandr," not a wandr guest.

## Re-examined 2026-06-18 — the blocker is "a WASI stdlib + one codegen flip," not a large effort

Prompted by the TeaVM `WEBASSEMBLY_GC_WASI` work shipping (see
`reference_flutter_go_ui_wandr` / task 113), re-derived the Dart blocker from
first principles instead of restating "big runtime floor." Correction: **the
floor is mostly already built**, and the remaining work splits into exactly two
bounded, upstream-owned pieces — **neither a stopper.** Upstream state unchanged
since the 2026-06-13 deep dive (#53884 / #54394 both open, same two named
blockers, still no wasmtime proof-of-life as of the May-2026 issue activity).

**1. A "WASI stdlib" (the big piece — but mostly done, and library-shaped).**
Removing the JS host means providing Dart's platform/runtime as in-wasm, JS-free
code. That is exactly the in-tree `dart2wasm_standalone_platform.dill`. Item by
item it is trivial-to-bounded, *not* primitives:

| #53884 item | reality | note |
|---|---|---|
| printing / current time | trivial — `fd_write` / `clock_time_get` (we wrote these for TeaVM) | in `.dill` |
| math / double→String | easy/bounded — libm; ryu/grisu | in `.dill` |
| timers / event loop / async | bounded — in-wasm microtask queue; **maps onto wandr's step-executor coroutine pump** | in `.dill` |
| stack traces / weak maps / finalizers | **degrade** (we degraded TeaVM stack traces too); Dart concedes partial | accepted |
| regex | heaviest single item — but see below | tree-shakeable |
| strings | in-wasm String variant (`stringref_platform.dill`) vs `wasm:js-string` | see codegen note |

  *Regex specifically:* `RegExp` is a `dart:core` **API backed by the irregexp
  engine** (a port of V8's), **not a language/word primitive**. In dart2js /
  dart2wasm-on-browser it is **host-imported** (the JS engine's `RegExp`, itself
  irregexp) — that import is the `dart:js_interop` dependency #53884 removes.
  Standalone needs the engine *in* the module (compile irregexp-C++ via
  Emscripten/LLVM — which emits correct `exnref` automatically — or reimplement in
  Dart). It is also **tree-shaken if unused**, so a Flutter UI guest may never
  pull it. So regex = "ship an existing engine, only if used," not a reinvention.

**2. One codegen flip — exception handling (#54394) — the only non-stdlib gate.**
Architecture point that kills the obvious shortcut: **dart2wasm is a
Dart-written compiler that lowers Dart → WasmGC directly; it does NOT compile the
C++ VM to wasm.** So the EH issue is not "C++ exceptions" and there is **no C++
exnref lowering to drop in** — it is dart2wasm's *own* emitter currently producing
**legacy** wasm EH (`try`/`catch`/`rethrow`) and needing the **new** opcodes
(`try_table`/`catch_all`/`exnref`/`throw_ref`). That transform is mechanical and
well-trodden (LLVM/Emscripten/V8/the spec all did it; simolus3 spells out the
mapping), but it lives in the **Dart compiler**, not in reusable C++.

  *Exceptions ≠ WASI.* EH is a **core-WebAssembly** proposal, so there is no "WASI
  recommendation." The W3C wasm CG + engines are unambiguous: **legacy EH is
  deprecated; the `exnref`/`try_table` model is the standard.** **Browsers keep
  legacy for back-compat** (V8 accepts both) — which is exactly why dart2wasm got
  away with emitting legacy and hasn't migrated. Standalone engines remove the
  crutch.

**Why this differs from TeaVM (the honest, checked reason — not "Dart is harder").**
On wasmtime 45 I verified the host **cannot shim either gate**: `-W` exposes
`exceptions` (exnref) with **no `legacy` flag** (and `wasm-tools` has no
legacy→exnref transform), and **no js-string-builtins** exists in any option group
(`-W/-O/-C/-S`). For TeaVM our blockers were inside a small forkable compiler
*and/or* host-providable, so we had cheap leverage; for Dart the two gates are in
dart2wasm's codegen + a stdlib `.dill`, both **host-unshimmable** and both
**actively being closed upstream by Google.** So *waiting strictly dominates
forking* — not because the work is large (the stdlib is mostly built; the EH flip
is small) but because our leverage is ~zero and the prize (Flutter's ecosystem)
lands without us.

**wandr readiness / action.** The device runtime already runs WasmGC + the new
`exnref` EH + function-references (the AOT flags enabled for Kotlin), so the moment
dart2wasm emits `exnref` standalone output, **wandr loads it with nothing new**.
Action stays: **track #53884 + #54394**; on landing, apply the proven Kotlin/TeaVM
pipeline (standalone module → WASI floor → `wasm-tools component new --adapt` →
WIT bindings). No fork.

## Rendering seam — one nuance the deep dive sharpened

Both shipping Flutter web renderers bundle **Skia INTO wasm** — CanvasKit
(Skia-wasm via JS-interop, ~1.5 MB) and the newer **skwasm** (compact Skia
on a worker thread, WasmGC-gated). So there is no pre-existing "remote/host
Skia" path to inherit; the retargetable seam is specifically **web_ui's
`dart:ui`→CanvasKit *binding* layer** (the JS-interop calls into the Skia
module). The wandr port replaces that binding with `wasi:canvas` WIT imports
— a real reimplementation of the canvas/paragraph/scene binding in Dart, but
exactly the move we made for skiko/Compose, and the 0.0.2 contract is
already shaped for it (mapping table below). The framework→engine boundary
(`dart:ui`) is the clean seam; skwasm's in-memory Skia coupling is *not* the
thing to target.

## dart:ui → wasi:canvas 0.0.2 architectural fit (added 2026-06-12)

Re-checked against the redesigned contract (`proposals/wasi-canvas/
REDESIGN-0.0.2.md`) so the answer is ready the day dart2wasm grows a
WASI target — Flutter is the prospective SIXTH reference consumer, and
its check found real contract gaps (now folded into 0.0.2; see below).

**Profile: managed-ui, the full stack — and the strongest validation of
`scene` yet.** Flutter's compositor speaks SceneBuilder: push
transform/offset/clip/opacity layers + `addPicture`. That is the
`scene` interface almost name-for-name (layers + pictures + replay-time
resolution), independently confirming that host-retained layers are the
managed-toolkit pattern, not Compose-private machinery:

| dart:ui surface | 0.0.2 home |
|---|---|
| Canvas (drawRect/RRect/DRRect/oval/circle/line/arc/path/paint/points) | `draw.canvas` verb-for-verb (SkCanvas heritage on both sides) |
| Picture / PictureRecorder | `graphics.start-recording` / `finish-recording` / `draw-picture` |
| SceneBuilder pushTransform/Offset | `scene.layer.set-transform` |
| SceneBuilder pushClipRect/RRect/Path | layer clip setters (clip-path added to `scene` from this check) |
| SceneBuilder pushOpacity | `scene.layer.set-alpha` |
| Paragraph / ParagraphBuilder (SkParagraph) | `layout` near 1:1 — same skparagraph ancestry; maxLines/ellipsis = the 0.0.2 builder setters |
| Path.combine | `draw.combine-paths` |
| ImageShader, Gradient.linear/radial/sweep | `graphics` factory (gradients' optional matrix4 = the `local` transform 0.0.2 gained from this check) |
| Image.toByteData / FragmentProgram / drawAtlas / drawVertices / drawShadow(path) / BackdropFilter / ShaderMask / placeholders | named deferrals, all with additive lanes (drawVertices rides egui's `draw-mesh` promotion) |

**Contract findings this check fed back into 0.0.2** (the value of
checking prospective consumers against the BREAKING-change classes now,
while the redesign is unwired):

1. **blend-mode must ship at the full 29-mode skia/CSS union** — enum
   case additions are instantiation-breaking just like record fields
   (R1 explicitly covers enums now). dart:ui uses all 29; Compose's
   binding silently degrades the missing 10 (dst, dst-over, src-in,
   src-out, plus, modulate, hue, saturation, color, luminosity) to
   src-over today — a latent fidelity bug the Flutter check exposed.
2. **text-style.shadows is a `list<text-shadow>`** (dart:ui TextStyle
   carries a shadow LIST; one optional shadow was Compose-snapshot
   thinking — the exact 0.0.1 mistake shape, caught pre-freeze).
3. **gradient constructors gain `local: option<transform>`** (function
   signatures are a breaking class too; dart:ui Gradient takes an
   optional matrix4, and skiko's `makeWithLocalMatrix` no-op stub was
   masking the same gap for Compose).

Everything else was already covered or already a named deferral. The
verdict stands: when dart-lang/sdk#56366 lands, the contract is ready;
the work is the web_ui-style binding (CanvasKit JS-interop → WIT
imports), not architecture.

## Final guest-UI lineup

| | Compose (Kotlin) | dioxus-canvas (Rust) | Slint (Rust) | Avalonia (C#) | Qt (C++) | **Flutter (Dart)** |
|---|---|---|---|---|---|---|
| Status | shipping | production | shipped (task 100) | shipped on device (task 107) | not practical | **blocked, gate closing** |
| wasi/component toolchain | hand-rolled (ours) | native | native | wasip1+adapter | nonexistent | **in progress (#53884, in-tree standalone platform)** |
| Render-seam fit | skiko (ours) | ours | ItemRenderer ✅ | IDrawingContextImpl ✅ | QPainter ✅ / Quick ✖ | **web_ui CanvasKit-binding seam ✅✅** |
| Text model | host-shaped | host-shaped | guest-shaped | guest-shaped | guest-shaped | **host-shaped (= our paragraph iface)** |
| Revisit gate | — | — | Slint 1.17 release | (shipped) | upstream wasi port | **dart-lang/sdk#53884 standalone (active)** |

## Addendum — the Go question: is there a Skia-backed Go UI?

**No — essentially none exists, which is why it can't be found.** Surveyed
2026-06-11:

- **go-skia** (go101): an old cgo Skia-binding experiment, dead.
- **unison** (richardwilkes): the one real, maintained Skia-via-cgo Go UI
  toolkit — desktop-only (glfw), and **cgo doesn't compile for wasm
  targets**, so it's unportable to wandr regardless.
- **Gio, Fyne, Cogent Core, Ebitengine**: all grew their own renderers
  (Gio: Pathfinder-style GPU vector renderer; Fyne: OpenGL; Cogent Core:
  own rasterizers). Go culture avoids cgo (cross-compilation pain), so a
  Skia-binding ecosystem never formed — UI authors wrote pure-Go pipelines
  instead.

If a Go guest UI is ever wanted, the study target is **Gio**, not a Skia
binding: pure Go (no cgo), already runs on browser wasm, and its text stack
(`go-text/typesetting`) is a pure-Go shaper producing glyph runs — the
Slint/parley model, fitting the task-100 `create-typeface`/`draw-glyphs`
verbs. The friction is the renderer (Gio's op-list feeds an integrated GPU
pipeline — a custom skiko-gfx backend means reimplementing its renderer at
the ops layer) and the Go-component toolchain (big Go = wasip1 + adapter;
TinyGo = native wasip2 but Gio doesn't support TinyGo). Verdict: possible,
Slint-or-more effort, no demand — recorded for the day someone asks.
