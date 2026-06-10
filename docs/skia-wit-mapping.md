# Skia ↔ `my:skiko-gfx` mapping — the canonical contract

> Written 2026-06-11 from the union of three real consumers. This doc is the
> compatibility contract for "any UI library with a Skia backend": if a
> library's Skia usage maps below, it can run as a wandr guest with only a
> guest-side adapter — no WIT changes.

## Principle

The WIT mirrors **Skia's draw-time semantics, value-encoded** — not Skia's
C++ object graph. The adaptation rule (which all three consumers converged on
independently):

- **Builder/setter objects stay guest-side**, serialized at the draw call:
  `SkPaint` → the flat `paint-attrs` record; `SkPath` → SVG path string;
  `SkMatrix` → `matrix-3x3` record / 9 floats.
- **Heavy long-lived objects stay host-side as u32 ids**: `SkImage`,
  `SkShader`, `SkTextBlob`, `SkPicture`, `SkTypeface`, paragraphs, offscreen
  surfaces. 0 is always the "not live / failed" sentinel.
- **Canvas state queries don't cross the boundary** (no
  `getLocalToDevice`/`getLocalClipBounds` verbs): adapters track their own
  matrix/clip stack guest-side, like Slint's femtovg renderer and the Kotlin
  binding both do.
- **paint-attrs is ABI-frozen.** Adding a field breaks every consumer
  including the hand-maintained Kotlin binding. A paint feature the record
  can't carry gets a *fused verb* instead (see draw-shadow-rrect).
- **Additive evolution only.** New verbs never break existing compiled
  guests (a component imports only the functions it calls; the host linker
  may offer a superset). Renames/signature changes are a v2 event we don't
  schedule.

## The three consumers

| Consumer | Adapter | Canvas verbs used |
|---|---|---|
| **Compose Multiplatform** (via skiko) | hand-maintained Kotlin binding (`external/skiko/.../generated/InternalSkikoUi.kt`) | ALL (~104) — it defined most of the surface |
| **dioxus-canvas** (`crates/dioxus-canvas`) | inline trimmed WIT in `launch.rs` | begin/end-frame, clear, save, restore, clip-rect, draw-rect/rrect, text blobs (create/draw/drop), create-image-from-encoded, draw-image-rect, get-image-width/height, surface-width/height + paragraph subset for measure |
| **Slint** (prospective — `i-slint-renderer-skia` inventory, master 1.17) | future `i-slint-renderer-wandr` (ItemRenderer impl, femtovg-style) | see mapping below; needs the 2026-06-11 additive batch |

## Mapping by Skia API area

Status: ✅ mapped · 🆕 added 2026-06-11 · 🚫 deliberately excluded/deferred.

### SkCanvas — state & transform
| Skia export | WIT verb | Notes |
|---|---|---|
| save / restore | `save` / `restore` | ✅ |
| saveLayerAlpha | `save-layer(x,y,w,h,has-bounds,alpha)` | ✅ opacity layers |
| translate / scale / rotate / skew / concat | same names | ✅ `concat` takes 9 floats row-major |
| resetMatrix | `reset-matrix` | ✅ resets to host base transform, not identity |
| getLocalToDevice / getLocalClipBounds | — | 🚫 track guest-side (see Principle) |

### SkCanvas — clip
| Skia export | WIT verb | Notes |
|---|---|---|
| clipRect / clipRRect | `clip-rect` / `clip-rrect` | ✅ Intersect; bc-* variants take `clip-mode` (Intersect/Difference) |
| clipPath | `clip-path(svg, aa)` | ✅ |

### SkCanvas — geometry draws
| Skia export | WIT verb | Notes |
|---|---|---|
| clear / drawPaint | `clear(argb)` / `draw-paint(paint)` | ✅ |
| drawRect / drawRRect / drawDRRect / drawOval / drawLine / drawArc | same names | ✅ |
| drawPath | `draw-path(svg, paint)` | ✅ SVG string; `Op(SkPathOp)` → `path-combine` |
| drawPoint(s) / drawLines / drawPolygon / drawCircle | `bc-draw-*` only | ✅ (main-canvas variants unneeded so far) |
| drawVertices | `bc-draw-vertices` | 🚫 no-op stub — no consumer renders vertices |
| drawAtlas / drawPatch / RuntimeEffect (SkSL) | — | 🚫 no consumer uses them |

### Text — three levels, all real Skia exports
| Skia export | WIT verb | Who uses it |
|---|---|---|
| **SkCanvas::drawGlyphs** (glyph ids + positions) | 🆕 `draw-glyphs(typeface-id, size, ids, positions, origin, paint)` + `bc-draw-glyphs` | Slint-class guests that shape in-guest (parley); glyph ids are valid only against the exact font binary → pair with create-typeface |
| **SkFontMgr::makeFromData** | 🆕 `create-typeface(bytes, index)` / `drop-typeface` | same; host builds the typeface from the SAME bytes the guest shaped with (never `match_family_style` — zero-metrics trap) |
| **SkTextBlob** (host-shaped from strings) | `create-text-blob`, `begin/add/end-text-blob`, `draw-text-blob` | Kotlin + dioxus-canvas: host shapes via SkShaper + system-font fallback |
| **SkParagraph** (the skparagraph module) | the whole `paragraph` interface | Compose Text layout + dioxus measure-text; mirrors upstream skiko's Paragraph API |
| variable-font axes (FontArguments) | — | 🚫 deferred until a guest ships a variable font |

### Images
| Skia export | WIT verb | Notes |
|---|---|---|
| Images::RasterFromData (RGBA8888) | `create-image(w, h, pixels)` | ✅ |
| Image::DeserializeFromEncoded | `create-image-from-encoded(bytes)` | ✅ PNG/JPEG/WebP/…; dims via get-image-width/height |
| drawImage / drawImageRect | `draw-image` / `draw-image-rect` | ✅ sampling fixed (Fast constraint); per-call SamplingOptions only on `create-image-shader` |

### Shaders & filters (paint effects)
| Skia export | WIT verb | Notes |
|---|---|---|
| GradientShader Linear/Radial/Sweep | `create-linear/radial/sweep-gradient` | ✅ → shader-id in paint-attrs |
| SkImage::makeShader | `create-image-shader(image, tiles, sampling, matrix)` | ✅ |
| Shaders::Blend | `create-blend-shader(mode, s1, s2)` | ✅ |
| Shaders::Color (solid) | — | 🚫 use paint color, or blend against a 1×1 image shader if ever needed |
| SkColorFilter Blend / invert | `paint-attrs.color-filter-kind/-color` | ✅ covers Slint's colorize-by-color (SrcIn) |
| **SkMaskFilter::MakeBlur** (box shadows) | 🆕 `draw-shadow-rrect(rect, radii, sigma, color)` + `bc-` twin | fused verb (paint-attrs frozen); Normal blur style; sigma<=0 = plain fill |
| ImageFilters (blend/image/shader compose) | — | 🚫 deferred — only Slint's colorize-by-*gradient* needs it (rare); colorize-by-color maps above |

### Pictures, drawables, offscreen surfaces
| Skia export | WIT verb | Notes |
|---|---|---|
| SkPictureRecorder / SkPicture | `create-picture-recorder`, `begin-picture-recording`, `finish-recording-as-picture`, `draw-picture` | ✅ recording mode reroutes all main-canvas draws |
| SkDrawable (live re-record) | `create-drawable`, `set-drawable-*`, `draw-drawable` | ✅ wandr's RenderNode analogue (Compose layers) |
| **SkSurface (offscreen raster)** | `create-bitmap-canvas(w,h)` + the `bc-*` verb family + `bitmap-canvas-snapshot` → image-id | ✅ this is `canvas.new_surface` for layers (Slint opacity/clip layers, Compose vector-icon rasterization) |

## The 2026-06-11 additive batch (🆕 above)

Six verbs, host impl in `runtime/wandr-host/src/canvas_impl.rs`
(`guest_typefaces` map + `prep_glyph_run` / `make_shadow` helpers):
`create-typeface`, `drop-typeface`, `draw-glyphs`, `bc-draw-glyphs`,
`draw-shadow-rrect`, `bc-draw-shadow-rrect`. They complete the surface a
Slint `ItemRenderer` needs (see `reference_slint_wasip2` memory for the
renderer plan). Existing compiled guests are unaffected; the Kotlin binding
needs no edit (it never calls them).

## Adding more

1. Check this doc — is it a real gap or does an adaptation cover it?
2. Additive verbs only; never touch `paint-attrs` or existing signatures.
3. Name 1:1 with the Skia export; document the value-encoding adaptation.
4. Mirror per the WIT sync rule (`docs/build-pipeline.md`) and update this
   doc's tables + the batch log above.
