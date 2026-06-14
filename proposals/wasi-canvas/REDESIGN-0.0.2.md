# wasi:canvas 0.0.2 — redesign proposal (NOT WIRED)

> Status: **proposal only** (2026-06-12, task 102 post-stage-3 review).
> Nothing in `wit/` changes with this document; no host or consumer code
> moves. This is the design that would let BOTH consumer classes —
> managed-runtime UIs (Compose/skiko-class) and native-shaper UIs
> (Slint/dioxus/chrome-class) — consume the package **without any further
> contract modification**, while staying inside WASI's design rules.

## 0. Design goal (the acceptance criteria, user-fixed 2026-06-12)

> **An architecturally clean design, without overlapping
> functionalities, acceptable to WASI, and 100% consumable by the
> reference UI libraries.**

Every section below serves one of those four clauses; §11 is the final
check of all four. "100%" means: every operation a shipped or
first-port renderer emits has a contract home — named deferrals are
rare ops that don't block a port (proven: Slint SHIPPED with the same
deferral class outstanding).

## 1. Why a redesign, and the method error it corrects

The 0.0.1 draft was derived from a **tooling snapshot**, not from the
owning source of truth. At draft time, "Kotlin stays on `my:skiko-gfx`"
(COMPATIBILITY.md) excluded the *maximal* consumer from the consumer set,
so `paint` shipped without color-filter, `line-metrics` shipped at 7 of 13
fields, and rect queries shipped without box styles — all features the
empirical union (`docs/skia-wit-mapping.md`, written FIRST) had already
proven necessary, several of them needed by *Slint* too (colorize). When
the snapshot went stale (the Kotlin wit-bindgen generator existed all
along), re-adding the trimmed fields was a **breaking record change** that
forced a rebuild of every consumer (the 2026-06-12 stage-3 event) — the
exact failure the mapping doc's own "paint-attrs is ABI-frozen / additive
evolution only" rule predicted.

**The corrected method, used throughout this document:**

1. The contract is derived from the union of **consumer semantics**
   (what Skia-backed UIs draw), never from consumer **toolchains**
   (which guests can bind today). Toolchains are the binding layer's
   concern and change under you.
2. Every exclusion is a **named, dated decision with its re-entry path
   stated** (§7) — not a margin note whose consequences freeze into
   record layouts.
3. Shapes are chosen so all future evolution is **additive** (§2).

## 1b. The reference libraries (the validation set)

Every contract decision in this package — additions, R5 cuts, record
shapes — is cross-checked against these FOUR reference consumers. They
span all positions on the ownership axes (§3), which is what makes the
set sufficient:

| Reference library | wandr adapter | Status | Axis positions |
|---|---|---|---|
| **Compose Multiplatform** (skiko-compose) | `external/skiko` wasmWasi binding | shipped; migrated through stages 1–3 | host-shapes (`layout`), host-retained scene (`scene`) |
| **dioxus** | `crates/dioxus-canvas` | shipped; migrated to 0.0.1 | host-shapes (`layout`), guest scene |
| **Slint** | `crates/slint-wandr` | shipped; the 0.0.1 proving consumer | guest-shapes (`glyphs`), guest scene |
| **Avalonia UI** | prospective (`docs/avalonia-wandr-feasibility.md`) | mapped out-of-sample (§9) | guest-shapes (`glyphs`), guest scene (in-guest compositor) |

A decision validated against fewer than all four is not validated
(the 0.0.1 lesson: it was validated bottom-up against the easy three
positions and broke on the fourth).

## 2. Evolution rules (binding for this package)

These are the rules that make "no further modification" a property of the
design rather than a hope:

- **R1 — breaking-class shapes ship at union size.** A record is
  ABI-frozen the moment one consumer compiles against it — and so are
  ENUMS (adding a case is instantiation-breaking) and FUNCTION
  SIGNATURES (adding a parameter likewise). All three ship at the union
  of known consumer semantics — including PROSPECTIVE consumers for the
  breaking classes, where checking costs nothing pre-freeze (the
  Flutter check, §9); speculative needs stay out (R3 covers them).
- **R2 — methods and free functions are the additive lane.** Resources
  grow methods, interfaces grow functions; compiled components import only
  what they call, so this never breaks anyone. Consequently: anything
  with a foreseeable option-space is shaped as **resource + setters**, not
  as a constructor record (see `paragraph-builder`).
- **R3 — a record change is a version event, never a patch.** If a record
  must grow, that is `wasi:canvas@0.0.(n+1)` — a NEW package version the
  host serves **simultaneously** with the old one over the same renderer
  (the proven my:skiko-gfx ⇄ wasi:canvas coexistence mechanic, applied to
  versions). Consumers migrate per-consumer or never; "rebuild the world"
  stops being a thing.
- **R4 — the binding floor.** Only shapes proven bindable by BOTH consumer
  classes are allowed: resources (own/borrow, imported-only), records,
  variants/options/results, enums, strings, lists, tuples. **No** WIT
  futures/streams/maps/fixed-length lists (unimplemented in the Kotlin
  generator), no exported interfaces in this package (import-only keeps
  the canvas neutral to the embedder's event model — the red line).
- **R5 — orthogonality: no derivable verbs.** If a candidate function is
  (1) semantically composable from existing primitives, (2) within the
  guest's CAPABILITY (a managed guest has no pathops library or shaper —
  capability gaps are host-side by definition), and (3) materially free
  (composition doesn't multiply hot-path wire calls or force a full
  list lift to answer a scalar), it stays OUT of the WIT and lives in
  guest adapters. Already-applied instances: circle→oval,
  points/polygon→rects/lines, skew/rotate-about-point→concat,
  restore-to-count→restore loop, opacity-mask→dst-in-in-layer,
  max-width→guest-remembered layout width. Kept-despite-derivability
  (failing test 2 or 3): combine-paths (capability), line-count
  (full-list lift to answer a scalar), did-exceed-max-lines (not
  derivable at all).

## 3. The ownership axes (why the package is split the way it is)

"Compose is host-driven, dioxus/slint are guest-driven" is really three
independent ownership questions, and the package puts an interface
boundary exactly where each can flip — a consumer declares its position
per axis by what it imports, instead of the contract baking one model in:

| Axis | Guest-owned form | Host-owned form | Where it lives |
|---|---|---|---|
| Frame driving | poll loop (wasi:surface-style) | reactor push (wandr render-frame) | **Outside the package** (red line) — the embedder's contract; any frame model can hand a guest a canvas. Push/pull reconciliation: `docs/surface-convergence-proposal.md` |
| Text shaping | `glyphs` (Slint/parley, Avalonia/HarfBuzz) | `layout` (Compose-class managed runtimes; also dioxus) | Two optional interfaces — the axis is per-consumer, not "Compose vs everyone" |
| Retained scene | re-issue draws each frame (`types`+`draw` is the whole world) | host-retained layers: record once, mutate transforms (`scene`) | `draw.picture` (floor, everyone) + optional `scene` (§4.5); a host without `scene` still runs the managed class via per-frame re-record — correct, not scroll-cheap |
| Rasterization | wasi-gfx (webgpu / frame-buffer contexts) | **always host here** — wasi:canvas's definition | Sibling context type on the same surface vocabulary (graphics-context idiom), not this package's fight |

The 0.0.1 mistake on this map: axis 3's host-owned form was mislabeled
"Compose-private machinery" and pushed outside the contract, when it is
the standard retained-layer pattern of every managed UI toolkit
(RenderNode, CALayer, Flutter layers). `scene` names it.

## 3b. Consumer profiles

Three world shapes, per WASI modularity (hosts may implement a subset of
*interfaces*; each interface is all-or-nothing):

| Profile | Imports | Consumers proven |
|---|---|---|
| **immediate** | `types`, `draw` | chrome guests, dioxus-canvas |
| **shaping-guest** | + `glyphs` | Slint (parley), Avalonia-class (HarfBuzz) |
| **managed-ui** | + `layout`, `scene` | Compose/skiko-class (host shapes, retained layers) |

`embedding` stays the non-normative reactor-host example (a wasi-gfx host
would substitute `wasi:surface` + graphics-context); input/windowing stay
out of the package entirely (COMPATIBILITY.md red line, unchanged).

## 4. The package, interface by interface (deltas vs 0.0.1-as-shipped)

### 4.1 `types` — paint unchanged; blend-mode completed to the union

Post-stage-3 `paint` is already at union size (style, color, alpha, blend,
anti-alias, shader borrow, stroke quad, mask-blur, color-filter).
**`blend-mode` grows from 19 to the full 29-mode skia/CSS set** (adding
dst, dst-over, src-in, src-out, plus, modulate, hue, saturation, color,
luminosity): enum cases are a breaking class (R1), dart:ui uses all 29,
and the Compose binding's "extended modes → src-over" fallback is a
latent fidelity bug this closes. (Dash/path-effects and runtime shaders
remain named deferrals — §7 — because no consumer has *shipped* the
need.)

### 4.2 `draw` — one additive function

```wit
enum path-op { union, intersect, difference, xor, reverse-difference }

/// Boolean path combination (SkPathOps class). Pure computation over
/// guest-owned data — capability-free by the same Capabilities.md
/// distinction as typeface.from-bytes. In the contract because it fails
/// R5's capability test: managed guests have no 2D boolean-ops library.
/// Compose's Path.op / clip-by-path difference flows need it;
/// SVG-string in, SVG-string out.
combine-paths: func(a: string, b: string, op: path-op) -> string;
```

`set-transform` was considered and REJECTED under R5: absolute
transforms are derivable by guest-side matrix tracking (mirror the save
stack; on set emit `concat(A⁻¹·T)` and anchor the tracked matrix to the
exact target, so inversion error never accumulates) — the adapter
pattern `docs/skia-wit-mapping.md` already prescribes and slint-wandr
already ships. The legacy `set/reset-matrix` residue retires into the
Kotlin adapter, not into the contract.

**Four-way cross-check of the cut (1b set, 2026-06-12):**

| | set-transform | one-shot draw-text |
|---|---|---|
| Compose | `setMatrix` rare (SkiaBackedCanvas edge paths) → adapter tracking | drawString/TextLine rare → paragraph mapping |
| dioxus | never sets absolute transforms | **the live proof**: dioxus-canvas already migrated legacy text-blobs → `layout` paragraphs, shipped + device-verified |
| Slint | never (tracks guest-side already — the shipped precedent) | never (glyphs) |
| Avalonia | **heaviest user**: `IDrawingContextImpl.Transform { get; set; }` is an absolute SETTER applied per visual (verified against AvaloniaUI/master 2026-06-12) — the derivation still passes R5 test 3 (one wire call either way; per-set cost is one guest-side 3×3 inverse-multiply), but Avalonia is the named first candidate to trigger the R2 re-entry if tracking shows drift or profile cost | never (DrawGlyphRun only) |

Re-entry lane for both cuts: additive method/function (R2).

One signature amendment (R1, from the Flutter check): the three
`graphics` gradient constructors gain a trailing `local:
option<transform>` (shader-local matrix, independent of canvas
transform). dart:ui's Gradient takes an optional matrix4, and skiko's
`makeWithLocalMatrix` no-op stub was silently masking the same gap for
Compose; signatures are breaking, so it ships now while 0.0.2 is
unwired.

### 4.3 `glyphs` — unchanged

The shaping-guest floor is complete (typeface-from-bytes + positioned
runs + paint with blur covers the legacy "additive batch" Slint needed).

### 4.4 `layout` — the union the managed-ui class actually ships

Two record upgrades (R1: union size NOW, in the version that introduces
them), the builder reshaped to the additive-forever form (R2), and one
new function.

```wit
interface layout {
    use types.{color, rect, point};
    use draw.{canvas};

    enum decoration-line-style { solid, double, dotted, dashed, wavy }

    /// Text decorations — Compose TextDecoration.{Underline,LineThrough}
    /// ships today (silently dropped by the current port).
    record decoration {
        underline:    bool,
        overline:     bool,
        line-through: bool,
        /// 0 = use the text color.
        color:        color,
        style:        decoration-line-style,
        /// Multiple of the font-suggested thickness.
        thickness:    f32,
    }

    /// Compose Shadow(color, offset, blurRadius) — ships today, dropped
    /// by the current port.
    record text-shadow {
        color:  color,
        offset: point,
        sigma:  f32,
    }

    /// UNION-SIZED (R1): every field a shipped managed-ui consumer
    /// emits. Optional sub-records keep the cheap case cheap.
    record text-style {
        family:         string,
        size:           f32,
        weight:         u32,
        italic:         bool,
        color:          color,
        letter-spacing: f32,
        /// Line height as a multiple of the font size; 0 = font default.
        line-height:    f32,
        /// Vertical run shift (superscript/subscript); 0 = none.
        baseline-shift: f32,
        decoration:     option<decoration>,
        /// Empty = none. A LIST because dart:ui carries multiple
        /// shadows per span — one optional shadow was Compose-snapshot
        /// thinking (the 0.0.1 mistake shape, caught pre-freeze).
        shadows:        list<text-shadow>,
        background:     option<color>,
    }

    enum align { start, center, end, justify }
    enum text-direction { ltr, rtl }

    // line-metrics, rect-height-style, rect-width-style, text-box:
    // unchanged from the post-stage-3 shapes (already union-sized).

    resource paragraph {
        // ... post-stage-3 surface unchanged ...
        /// True when max-lines truncated the layout (R2 addition).
        did-exceed-max-lines: func() -> bool;
    }

    /// Reshaped to setters (R2): paragraph-wide options arrive as
    /// methods, so the next option (strut, tab policy, …) is additive
    /// instead of a constructor break. This is the structural fix for
    /// the class of mistake stage 3 paid for.
    resource paragraph-builder {
        new:           static func(default-style: text-style)
                       -> paragraph-builder;
        set-align:     func(a: align);
        set-direction: func(d: text-direction);
        /// 0 = unlimited.
        set-max-lines: func(n: u32);
        set-ellipsis:  func(e: string);
        push-style:    func(style: text-style);
        pop-style:     func();
        add-text:      func(text: string);
        build:         static func(b: paragraph-builder) -> paragraph;
    }
}
```

A one-shot `draw-text` convenience was considered and REJECTED under
R5: a host-shaped run IS a single-style paragraph painted at
`baseline-origin.y − alphabetic-baseline()`, and the adapter caches the
paragraph exactly where the legacy blob cache lived (the whole
create/draw/drop + begin/add/end text-blob family retires into this
mapping). Re-entry lane if per-run paragraph churn ever profiles hot:
additive function (R2).

### 4.5 `scene` — NEW optional interface (retained layers)

The piece that makes the managed-ui class *first-class* instead of
half-resident on a private package. This is the standardized form of
wandr's `WasiDrawable`/RenderNode machinery — which is not
Compose-private at all: it is the retained-layer pattern every managed UI
toolkit ships (Android RenderNode, CALayer, Flutter layers), and it is
what makes scrolling cost a transform write instead of a re-record.

```wit
/// OPTIONAL host-retained layers. Embedders MAY omit it (the canonical
/// feature test is instantiation-time, like `layout`); a guest that
/// can't import it re-records via `draw.picture` each frame — correct,
/// just not scroll-cheap.
interface scene {
    use types.{transform, rect, rounded-rect};
    use draw.{canvas, graphics, picture};

    resource layer {
        /// Created through the embedder-granted factory (capability
        /// rule: all host-resident allocation flows through a handle).
        new: static func(g: borrow<graphics>) -> layer;

        /// Set the layer's content by CONSUMING a recording canvas
        /// (graphics.start-recording). IMPLEMENTATION-DISCOVERED FIX
        /// (2026-06-12, path B): content must be finished as LIVE
        /// drawable state, not a picture — a picture SNAPSHOTS captured
        /// child layers, freezing them when a parent re-records; the
        /// recording form keeps nested layers resolving at replay time
        /// (skia finish-recording-as-drawable, the semantics the legacy
        /// RenderNode machinery already relies on). `set-picture` was
        /// dropped: R5-derivable from this (draw the picture into a
        /// one-op recording). Traps on a non-recording canvas.
        set-content:           func(recording: canvas);
        set-bounds:            func(bounds: rect);
        set-transform:         func(t: transform);
        set-clip-rect:         func(r: rect, anti-alias: bool);
        set-clip-rounded-rect: func(rr: rounded-rect, anti-alias: bool);
        /// SVG path data (Flutter ClipPathLayer; Compose outline clips).
        set-clip-path:         func(path: string, anti-alias: bool);
        clear-clip:            func();
        set-alpha:             func(alpha: u8);
        /// Material-style drop shadow from layer geometry; 0 = none.
        set-shadow-elevation:  func(elevation: f32);
    }

    /// Draw the layer's current content with its current properties.
    /// THE LOAD-BEARING SEMANTICS (the part that must be specified, not
    /// implied): when called on a RECORDING canvas, the RECORDING
    /// retains the layer host-side (never the guest's borrow) —
    /// replaying the finished picture resolves the layer's picture and
    /// properties AT REPLAY TIME, property/content updates never
    /// invalidate capturing recordings, and dropping the guest's layer
    /// handle does not invalidate pictures that captured it: the host
    /// keeps the layer alive until the last capturing picture drops.
    /// (Provenance: wandr's swappable-inner SkDrawable + its host-side
    /// refcount — the rule the legacy impl already enforces; this
    /// deferred resolution is what makes parent caching + cheap scroll
    /// coexist.)
    draw-layer: func(canvas: borrow<canvas>, l: borrow<layer>);
}
```

### 4.6 `embedding` — unchanged

Canvas-context (graphics / get-current-buffer / present), still
non-normative, still aligned with the wasi-gfx graphics-context idiom.

### 4.7 Diagnostics

Legacy `log-message` does NOT move in: that's `wasi:logging`'s job.
Guests import the standard interface; the package stays drawing-only.

## 5. The completeness proof (legacy → 0.0.2 home)

The check 0.0.1 skipped. Every `my:skiko-gfx` canvas + paragraph verb a
consumer calls today, and where it lives in 0.0.2 — if a row had no home,
the design would be incomplete:

| Legacy surface (consumer-proven) | 0.0.2 home |
|---|---|
| begin/end-frame, surface-w/h | `embedding` canvas-context; window metrics are the embedder's (window) contract |
| save / save-layer / restore | `draw.canvas` (unchanged) |
| translate/scale/rotate/skew/concat | `draw.canvas` (skew = concat) |
| set-matrix / reset-matrix | guest-side matrix tracking in the adapter (R5; the skia-wit-mapping principle, shipped in slint-wandr) |
| clip-rect / clip-rrect (per-corner) / clip-path | `draw.canvas` |
| clear, draw-paint/rect/rrect/drrect/oval/line/arc/path/point(s)/circle | `draw.canvas` (point/circle = guest-side mappings, as shipped) |
| path-combine (SkPathOps) | **`draw.combine-paths`** (§4.2) |
| draw-vertices | named deferral (no-op in every shipped consumer) — §7 |
| gradients, blend-shader, image-shader | `draw.graphics` factory |
| color-filter (blend/invert) | `types.paint.filter` (stage 3) |
| draw-shadow-rrect (fused) | `paint.blur` on draw-rounded-rect (already superseded in shipped ports) |
| create-image(-from-encoded), draw-image(-rect), image w/h | `types.image` + `draw.graphics` + `draw.canvas` |
| bitmap-canvas + ~30 `bc-*` twins + snapshot | offscreen `canvas` resource (one type, zero twins) + `snapshot` |
| pictures (recorder family) | `graphics.start-recording` / `canvas.finish-recording` / `draw-picture` |
| **drawables (create/set-*/draw/drop)** | **`scene.layer`** (§4.5) |
| text blobs (create/draw/drop, begin/add/end, draw-string) | adapter-cached `layout` paragraphs painted at baseline − alphabetic-baseline (R5) |
| paragraph builder + styles | `layout.paragraph-builder` (setter form; decoration/shadow/background now carried) |
| layout/paint/height/intrinsics/baselines/line-count | `layout.paragraph` |
| rects-for-range + modes + direction | `selection-boxes` (stage 3) |
| glyph-position / word-boundary | `offset-at` / `word-boundary` |
| line metrics ×13 (two-stage getters) | `lines()` list<record> (stage 3) |
| max-lines / ellipsis / didExceedMaxLines (stubbed today) | builder setters + `did-exceed-max-lines` (§4.4) |
| glyph runs + guest typefaces (Slint batch) | `glyphs` (unchanged) |
| log-message | `wasi:logging` |

Residue intentionally NOT in the package (correct layering, both worlds):
window metrics/density, IME, lifecycle, theme, frame pacing — sibling
interfaces next to the surface, exactly as `my:skiko-gfx` carries them
next to its canvas today.

## 6. WASI-rules audit (delta over COMPATIBILITY.md's, which still holds)

**Verified 2026-06-12 (re-run after the §9-driven amendments — 29-mode blend enum, shadows list, gradient local transforms, scene.set-clip-path, and the deferred draw-mesh shape):** (a) the complete 0.0.2 package — including
`scene`, the union records with nested `option<record>`s, the setter
builder, and a `managed-ui-guest` world — was materialized and passes
`wasm-tools component wit` (the component model's static rules:
resource/borrow placement, types, worlds); (b) the four design-principle
citations below were re-checked verbatim against
`WebAssembly/WASI@main` DesignPrinciples.md. Handle lifetimes are a
component-model rule, not a WASI-doc rule: **a `borrow` cannot outlive
the call** — which is why every retained relationship in `scene` is
specified as the HOST retaining its own internal references
(picture content, layer objects), never the guest's handle; host-retained
per-resource state is the norm across WASI (wasi:http bodies,
wasi:webgpu devices).

- **Capabilities / no ambient authority**: `scene.layer` is created via
  the `graphics` factory borrow; `combine-paths` and `draw-text` are pure
  computation / draw-through-handle. No new ambient names.
- **Modularity**: `scene` and `layout` are optional interfaces selected
  by worlds — the prescribed mechanism for "not appropriate for every
  host" (a CLI rasterizer host implements neither).
- **Partial-host implementability**: blessed explicitly; the canonical
  feature test stays instantiation-time.
- **Interposition**: everything remains plain funcs + resources; a
  wasm interposer can implement `scene` ON TOP of `draw` (re-record on
  property change) — the principle's "implementable by a component"
  test passes by construction.
- **Reactor neutrality (red line)**: still no events, no windowing, no
  guest loop anywhere in the package.

## 7. Named deferrals (each with its re-entry path)

Per the method fix: every exclusion is a dated decision, challengeable at
the design surface, with the lane it re-enters through.

| Deferral | Why (2026-06-12) | Re-entry lane |
|---|---|---|
| Dash / path effects (= W3C Canvas2D `setLineDash`/`lineDashOffset`) | no shipped consumer (Slint borders shipped solid). **The single clean additive gap vs W3C Canvas2D** — verified 2026-06-14, the only genuinely-absent Canvas2D drawing feature (others are covered or guest-side; see docs/egui-wandr-feasibility.md) | `paint` field (`dash: option<...>`, skia `SkDashPathEffect`) → **R3 version event**; flag the moment a consumer ships dashes |
| Foreground paint on text spans (gradient text) | rare; Compose ships color spans only | additive `push-style-with-paint` method (R2) |
| Placeholders / inline content | stubbed empty in the shipped Compose port | additive builder method + `placeholder-boxes` query (R2) |
| draw-vertices / atlas / patch / SkSL | no consumer renders them | additive verbs (R2) |
| Variable-font axes | no guest ships a variable font | additive `glyphs` params via new func (R2) |
| Image pixel readback / encode | privacy posture kept; no consumer needs it | additive `image` methods (R2) |
| Font-metrics-without-paragraph | paragraph intrinsics cover shipped needs | additive `layout.measure-text` (R2) |
| Layer effects (blur/drop-shadow on arbitrary content — Avalonia PushEffect, Compose Modifier.blur, Flutter BackdropFilter/ShaderMask) | only effect-on-*shape* shipped (paint.blur) | additive `save-layer-with-filter` method (R2) |
| Textured triangle meshes (`draw-mesh`) | no shipped consumer; egui is the promoting candidate (its ENTIRE output — docs/egui-wandr-feasibility.md) | additive function (R2); passes R5 as a primitive (not derivable at sane wire cost); shape pre-validated (wasm-tools, 2026-06-12); promotion must spec index validation (out-of-range → trap) |
| Image sub-region updates (`image.write-region`) | images immutable; egui's incremental font atlas is the promoting candidate (whole-texture re-upload meanwhile) | additive method (R2) — BUT it makes images mutable, so promotion must specify capture semantics for `image-pattern` shaders and pictures referencing the image (snapshot-at-creation vs live); until specified, determinism/interposition argue for snapshot |
| drawAtlas / drawShadow-on-path (dart:ui) | Flutter-only so far | additive verbs (R2) |

## 8. Adoption sketch (non-binding)

0.0.1 consumers are untouched — R3's coexistence means 0.0.2 is a second
`add_to_linker` over the same renderer, and migration is per-consumer:
the Kotlin world adopts `scene` + `draw-text` + `set-transform` and drops
its last `my:skiko-gfx` canvas/paragraph imports; Rust guests regenerate
whenever convenient. `my:skiko-gfx` then shrinks to the true remainder
(window/IME/lifecycle/theme + transport for input), which is the sibling-
package layering both worlds already use.

## 9. Out-of-sample validations (2026-06-12)

Three prospective consumers checked; per-row mappings live in their
memos — `docs/avalonia-wandr-feasibility.md` (below),
`docs/egui-wandr-feasibility.md` (mesh+atlas class; promotes the
draw-mesh deferral), `docs/flutter-wandr-feasibility.md` §dart:ui
(managed-ui class; independently re-derived `scene` via SceneBuilder and
drove the R1 breaking-class amendments: 29-mode blend enum, shadows
list, gradient local transforms).

### 9a. Avalonia

The design must hold for consumers that did NOT shape it. Avalonia is
the fourth class candidate (C#/.NET, HarfBuzz shaping, in-guest
compositor — `docs/avalonia-wandr-feasibility.md`), and every op in its
`IDrawingContextImpl` inventory has a 0.0.2 home through the
**shaping-guest profile** (`types`+`draw`+`glyphs`), with zero contract
modification:

- Geometry/clips/layers/gradients/images/tile-brushes map 1:1 (conic =
  sweep-gradient; render targets = offscreen canvas + snapshot;
  box-shadows = paint.blur).
- **PushOpacityMask — a "gap" in the legacy mapping — is expressible
  here**: save-layer → content → mask drawn with `blend = dst-in` →
  restore. 0.0.2's faithful blend pipeline composes what the legacy
  contract needed a verb for.
- Its in-guest compositor means it never imports `scene` (axis-3
  guest-owned) and never imports `layout` (it shapes) — validating that
  both are genuinely optional rather than Compose-shaped requirements.
- It would eventually press on exactly two named deferrals — pen dashes
  (guest-side pre-dashing meanwhile; §7 R3 lane) and layer effects
  (PushEffect; §7 additive lane) — neither a record break.

## 10. One open contract question (carried, not hidden)

**Text offset encoding.** The draft says UTF-8 byte offsets; the shipped
host passes offsets through to skparagraph; the Kotlin adapter passes
Compose's UTF-16 units through unchanged. ASCII masks the mismatch today.
0.0.2 must pick one normatively (UTF-8 byte offsets, matching `string`'s
canonical encoding) and the Kotlin adapter must convert at the boundary —
a host-impl/adapter correctness item discovered by the stage-3 source
read, recorded here so it's resolved by decision rather than by accident.

## 11. Overlap audit + final acceptance check (2026-06-12)

### 11a. Overlap audit — no two verbs doing the same job

Apparent pairs examined; each is LAYERING or an AXIS, except two
documented R5 exceptions:

| Pair | Verdict |
|---|---|
| `glyphs` vs `layout` (both draw text) | NOT overlap — the text-shaping axis; disjoint consumer classes, a world imports one (the 0.0.1 two-text-layers rule, re-affirmed) |
| `picture` vs `scene.layer` | layered — a layer REFERENCES a picture; scene adds only mutability + replay-time resolution |
| `save-layer(alpha)` vs `layer.set-alpha` | axis — scoped compositing while drawing vs retained property captured by recordings |
| `paint.blur` vs `layer.set-shadow-elevation` | justified pair — mask blur is per-geometry; elevation shadows derive from the LAYER's clip outline (ambient+spot model) and must live in the retained node so capturing recordings keep them |
| `draw-image` vs `image-pattern` shader | standard 2D split — composite op vs fill source (every 2D API has both) |
| offscreen+`snapshot` vs `picture` | different cost models — raster cache (icons) vs vector replay (scenes); consumers use both for different content |
| `clear(color)` vs `draw-paint{blend: src}` | **documented R5 exception** — semantically derivable, kept because it is the universal canvas idiom AND the host's distinct fast path (GPU clear); the one deliberate redundancy in the package |
| `embedding` vs `frame-handler` | different packages, complementary — canvas acquisition vs frame driving |

Net: zero unjustified overlaps; one idiom exception (`clear`) and one
retained/immediate shadow pair, both with recorded rationale.

### 11b. Final acceptance check

| Criterion | Status | Evidence |
|---|---|---|
| Architecturally clean | **PASS** | ownership axes (§3) put an interface boundary wherever ownership flips; profiles = worlds (§3b); R1–R5 make evolution additive-by-construction; platform remainder (window/IME/lifecycle) correctly OUTSIDE the package |
| No overlapping functionality | **PASS** | §11a — layering/axes throughout; two justified, documented exceptions |
| WASI-acceptable | **PASS (design level)** | full package machine-validated (wasm-tools, incl. all §9 amendments); DesignPrinciples re-verified verbatim against WASI@main; capabilities via the graphics factory; optional interfaces per modularity; interposition holds (scene implementable over draw); borrow-lifetime semantics specified host-retained. Formal acceptance = the phase-0→3 campaign (champion, portability criteria, 2 implementations) — a process question, not a design gap |
| 100% reference-library consumable | **PASS** | Compose: full, incl. `scene` (= its RenderNode) — stages 1–3 shipped the geometry+text half; dioxus: already shipped on the draft; Slint: the shipped proving consumer; Avalonia: every IDrawingContextImpl op mapped (§9a), first-port complete with the same rare-deferral class Slint shipped with. Prospects beyond the set: egui = one additive promotion (`draw-mesh`); Flutter = contract-ready, gated only on dart2wasm WASI |

**Companion check**: `wasi:input-handlers` gets the same treatment in
`proposals/wasi-input-handlers/REDESIGN-0.0.2.md` — its 0.0.1
pointer-event FAILED the six-consumer union (buttons/device/tilt/
enter-leave, all breaking-class) and is fixed there; both packages bump
together in the single path-B migration event, after which BOTH are
frozen: all reference libraries consume the same two contracts, and
nothing changes later except by additive method (R2) or deliberate
side-by-side version (R3).
