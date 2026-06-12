# wasi:canvas 0.0.2 — redesign proposal (NOT WIRED)

> Status: **proposal only** (2026-06-12, task 102 post-stage-3 review).
> Nothing in `wit/` changes with this document; no host or consumer code
> moves. This is the design that would let BOTH consumer classes —
> managed-runtime UIs (Compose/skiko-class) and native-shaper UIs
> (Slint/dioxus/chrome-class) — consume the package **without any further
> contract modification**, while staying inside WASI's design rules.

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

## 2. Evolution rules (binding for this package)

These are the rules that make "no further modification" a property of the
design rather than a hope:

- **R1 — records ship at union size.** A record is ABI-frozen the moment
  one consumer compiles against it. Every record below carries the full
  field set any *shipped* consumer semantics needs; speculative fields are
  NOT included (R3 covers them).
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

### 4.1 `types` — unchanged

Post-stage-3 `paint` is already at union size (style, color, alpha, blend,
anti-alias, shader borrow, stroke quad, mask-blur, color-filter). No
deltas. (Dash/path-effects and runtime shaders remain named deferrals —
§7 — because no consumer has *shipped* the need.)

### 4.2 `draw` — two additive functions

```wit
resource canvas {
    // ... 0.0.1 surface unchanged ...

    /// Replace the transform accumulated since this canvas was acquired
    /// (buffer, offscreen, or recording). The embedder's base transform
    /// (orientation, density policy) sits OUTSIDE this space and cannot
    /// be escaped — capability-clean absolute transforms, HTML canvas
    /// `setTransform` precedent. Closes the legacy `set/reset-matrix`
    /// residue (the last my:skiko-gfx canvas verbs Compose still calls).
    set-transform: func(t: transform);
}

enum path-op { union, intersect, difference, xor, reverse-difference }

/// Boolean path combination (SkPathOps class). Pure computation over
/// guest-owned data — capability-free by the same Capabilities.md
/// distinction as typeface.from-bytes. Compose's Path.op / clip-by-path
/// difference flows need it; SVG-string in, SVG-string out.
combine-paths: func(a: string, b: string, op: path-op) -> string;
```

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
        shadow:         option<text-shadow>,
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

    /// One-shot host-shaped run, baseline-anchored: the honest form of
    /// the legacy text-blob family (create/draw/drop, begin/add/end).
    /// Single-style runs draw directly; multi-run blobs are N calls;
    /// guests that re-draw shaped text cache a paragraph instead.
    draw-text: func(canvas: borrow<canvas>, style: text-style,
                    text: string, baseline-origin: point);
}
```

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

        /// The layer's content. The BORROW is not retained (component
        /// model rule: a borrow cannot outlive the call) — the HOST
        /// retains the immutable picture content internally, so the
        /// guest may drop its picture handle afterwards. Swapping
        /// content does NOT invalidate anything that has captured this
        /// layer (see draw-layer).
        set-picture:           func(p: borrow<picture>);
        set-bounds:            func(bounds: rect);
        set-transform:         func(t: transform);
        set-clip-rect:         func(r: rect, anti-alias: bool);
        set-clip-rounded-rect: func(rr: rounded-rect, anti-alias: bool);
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
| set-matrix / reset-matrix | **`canvas.set-transform`** (§4.2) |
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
| text blobs (create/draw/drop, begin/add/end, draw-string) | **`layout.draw-text`** (§4.4) + cached paragraphs |
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

**Verified 2026-06-12:** (a) the complete 0.0.2 package — including
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
| Dash / path effects | no shipped consumer (Slint borders shipped solid) | `paint` field → **R3 version event**; flag the moment a consumer ships dashes |
| Foreground paint on text spans (gradient text) | rare; Compose ships color spans only | additive `push-style-with-paint` method (R2) |
| Placeholders / inline content | stubbed empty in the shipped Compose port | additive builder method + `placeholder-boxes` query (R2) |
| draw-vertices / atlas / patch / SkSL | no consumer renders them | additive verbs (R2) |
| Variable-font axes | no guest ships a variable font | additive `glyphs` params via new func (R2) |
| Image pixel readback / encode | privacy posture kept; no consumer needs it | additive `image` methods (R2) |
| Font-metrics-without-paragraph | paragraph intrinsics cover shipped needs | additive `layout.measure-text` (R2) |

## 8. Adoption sketch (non-binding)

0.0.1 consumers are untouched — R3's coexistence means 0.0.2 is a second
`add_to_linker` over the same renderer, and migration is per-consumer:
the Kotlin world adopts `scene` + `draw-text` + `set-transform` and drops
its last `my:skiko-gfx` canvas/paragraph imports; Rust guests regenerate
whenever convenient. `my:skiko-gfx` then shrinks to the true remainder
(window/IME/lifecycle/theme + transport for input), which is the sibling-
package layering both worlds already use.

## 9. One open contract question (carried, not hidden)

**Text offset encoding.** The draft says UTF-8 byte offsets; the shipped
host passes offsets through to skparagraph; the Kotlin adapter passes
Compose's UTF-16 units through unchanged. ASCII masks the mismatch today.
0.0.2 must pick one normatively (UTF-8 byte offsets, matching `string`'s
canonical encoding) and the Kotlin adapter must convert at the boundary —
a host-impl/adapter correctness item discovered by the stage-3 source
read, recorded here so it's resolved by decision rather than by accident.
