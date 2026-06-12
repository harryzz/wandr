# Flutter/Dart on wandr — feasibility memo (+ the Go UI question)

> Written 2026-06-11, last entry of the guest-UI survey (Slint shipped /
> Avalonia feasible / Qt no — see the sibling memos). Grounded in Flutter's
> documented web architecture + dart-lang/sdk upstream state.
> Status: **analysis only — blocked on toolchain, architecture favorable.**

## Verdict

**Not practical today — but for the opposite reason from Qt.** Flutter's
*architecture* is unusually well-shaped for wandr (its web engine already
implements `dart:ui` in Dart over a remote Skia-command interface — the
skiko trick, shipped by Google), but the *toolchain* dead-ends: dart2wasm
emits WasmGC modules that only run in JS environments; a WASI target is an
open, unimplemented proposal
([dart-lang/sdk#56366](https://github.com/dart-lang/sdk/issues/56366)).
Even Kotlin/Wasm — whose component pipeline wandr had to hand-roll with a
forked adapter — at least *has* a WASI target; Dart does not. The gate is
that one issue: if dart2wasm ever grows `-t wasi`, Flutter jumps to the
front of the port queue. Until then: **don't.**

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

## The blocker, precisely

dart2wasm's output imports its platform from **JavaScript**: core-library
primitives (string helpers, typed-data, timers, event loop) are JS
builtins/imports, and the docs state the output "currently doesn't support
execution in standard Wasm runtimes like wasmtime." A WASI flavor means
Google (or a heroic fork) re-platforming the Dart wasm runtime libraries —
the same class of work as Kotlin/Wasm's `wasmWasi` target, which exists
because JetBrains built it. Hand-rolling that ourselves is bigger than
everything the Kotlin pipeline cost (adapter fork, stdlib pin, KT-86415),
against a moving runtime we don't control. **Not our fight.**

The other integration shape — running the native C++ Flutter engine
host-side via the embedder API (the flutter-pi model) — is rejected on
principle: it's a fat per-app native runtime with its own GPU context,
bypassing the wasm sandbox, the component model, and `my:skiko-gfx`
entirely. That's "Flutter beside wandr," not a wandr guest.

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
| Status | shipping | production | shipped (task 100) | feasible (2–4 wk) | not practical | **blocked on toolchain** |
| wasi/component toolchain | hand-rolled (ours) | native | native | preview | nonexistent | **nonexistent (open proposal)** |
| Render-seam fit | skiko (ours) | ours | ItemRenderer ✅ | IDrawingContextImpl ✅ | QPainter ✅ / Quick ✖ | **web_ui precedent ✅✅** |
| Text model | host-shaped | host-shaped | guest-shaped | guest-shaped | guest-shaped | **host-shaped (= our paragraph iface)** |
| Revisit gate | — | — | Slint 1.17 release | componentize-dotnet 1.0 / need | upstream wasi port | **dart-lang/sdk#56366** |

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
