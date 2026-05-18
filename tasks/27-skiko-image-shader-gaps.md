# Task 27 — Skiko WIT-shaped gaps: Image loading + shader variants

> **Status: ✅ device-verified 2026-05-18.** Implemented the skiko stubs
> that follow the existing WIT-handle pattern: `Image.makeFromEncoded`,
> `Image.makeShader`, `Shader.makeSweepGradient`, `Shader.makeBlend`,
> and the `Gradient`-object overloads of `makeLinearGradient` /
> `makeRadialGradient` / `makeSweepGradient`. `Bitmap.makeShader` was
> deferred (no host-side Bitmap state) — kept throwing with a
> doc-comment explaining the deferral. All other stubs in
> `SkiaTypes.wasi.kt` were replaced.
>
> **What landed:**
> - WIT: 4 new verbs (`create-image-from-encoded`,
>   `create-sweep-gradient`, `create-image-shader`, `create-blend-shader`)
>   + 2 query verbs (`get-image-width`, `get-image-height`, used so
>   `Image.makeFromEncoded` can return the correct dimensions to the
>   Kotlin `Image(id, width, height)` constructor) + 2 new records
>   (`matrix-3x3`, `sampling-options`).
> - Host (`canvas_impl.rs`): 6 new impl methods + helpers
>   (`sampling_options_from_wit`, `matrix3x3_from_wit`,
>   `blend_mode_from_wit`).
> - Kotlin bindings hand-edited in `generated/SkikoUi.kt` +
>   `generated/InternalSkikoUi.kt` (wit-bindgen 0.53.1 wasn't installed
>   locally). Pattern-matched against the existing `createLinearGradient`
>   for the direct calls; `createImageShader` uses indirect (>16 flat
>   params) with a 56-byte spill struct.
> - `SkiaTypes.wasi.kt`: 7 throw-stubs replaced with WIT delegations;
>   `SkiaTypes2.wasi.kt` Bitmap.makeShader stubs annotated as deferred.
> - Smoke card on device (`Task27SmokeCard.kt`) shows the four new
>   calls' status + a sweep / linear / radial Brush strip — visually
>   confirmed on Pixel 2 XL.
>
> **ABI choice: `option<matrix-3x3>` dropped to bare `matrix-3x3`.**
> The original WIT had `option<matrix-3x3>` for the `localMatrix`
> parameter, but no other verb in this file uses `option<record>` so
> wit-bindgen 0.53.1's emitted convention isn't in the existing
> generated code to copy from. Switched to always passing a matrix
> (identity = no-op on the host) which made the canonical-ABI marshaler
> straightforward. Kotlin callers passing `localMatrix = null` get
> `Matrix3x3.IDENTITY` on the wire.
>
> **ABI choice: `result<u32, string>` → bare `u32` (0 on failure).**
> Same reason — no existing `result`-returning verb in this WIT to
> copy. `Image.makeFromEncoded` returns 0 on decode failure; the
> Kotlin wrapper throws if it sees 0.
>
> These are all the same shape architecturally — add WIT verb →
> host-side `skia_safe` impl → Kotlin wrapper that replaces the
> `error(...)` body with the WIT call. Grouping them so the fresh
> session can land them in a single rebuild + redeploy cycle.
>
> Companions:
> - `wit/skiko-gfx.wit` — current WIT source of truth (mirror in
>   `skiko/skiko/wit/skiko-gfx.wit` after every edit per CLAUDE.md's
>   WIT-sync rule).
> - `wart-host/src/canvas_impl.rs` — Rust host implementations of
>   existing canvas/shader WIT functions (the pattern to copy).
> - `skiko/skiko/src/wasmWasiMain/kotlin/generated/SkikoUi.kt` and
>   `InternalSkikoUi.kt` — generated WIT bindings (hand-edit to add
>   new functions if `wit-bindgen` isn't re-run).
> - `tasks/11-gradient-shaders.md` — original linear/radial gradient
>   work. The same handle-based approach (`u32` shader id, stored in
>   a `HashMap<u32, skia_safe::Shader>` on host) extends to all the
>   stubs in this task.
> - `tasks/12-image-rect.md` — existing image-rect drawing. We
>   already have host-side `HashMap<u32, skia_safe::Image>`; this
>   task adds the *constructor* that populates it from encoded
>   bytes.
> - [[canvas-stub-noop-traps-compose]] (memory) — sister task 28
>   covers the OTHER 42 Canvas stubs (not in this task's scope).

## What this task is and isn't

**Is:** seven small additions to the existing canvas/shader WIT
surface, each one following the established
`create-X(...) → u32` + `drop-X(id: u32)` handle pattern. No new
abstractions, no architectural choices — just plumbing.

**Isn't:** wiring the abstract `org.jetbrains.skia.Canvas` class to
host-side skia (the 42 throw-stubs). That's task 28.

## The seven items

Listed in priority order (most-asked-for first):

### 1. `Image.makeFromEncoded(encoded: ByteArray): Image`

**Current state:** `SkiaTypes.wasi.kt:548` throws.
**Why we want it:** any wart-app that wants to load a PNG/JPEG from
bytes (asset, network, clipboard image-paste, decoded camera frame)
needs this. Currently impossible.

**WIT shape:**
```wit
create-image-from-encoded: func(bytes: list<u8>) -> result<u32, string>;
```

Returns a fresh image id, or an error message if decode failed.
Pairs with existing `drop-image(id: u32)`.

**Host-side:** `skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&bytes))` →
`Some(image)` → store in `state.images.insert(id, image)`. Error
case (e.g. unrecognized format) returns `Err`.

**Kotlin-side:** `SkiaTypes.wasi.kt` line 548 becomes:
```kotlin
fun makeFromEncoded(bytes: ByteArray): Image {
    val id = Canvas.Import.createImageFromEncoded(bytes)
        ?: error("Image.makeFromEncoded: decode failed on wasmWasi")
    return Image(id)
}
```

`Image` class on wasi already holds an `id: u32`. No structural
change needed.

### 2. `Shader.makeSweepGradient(...)`

**Current state:** lines 571 + 578 throw.
**Why we want it:** Compose's circular indicators and arc helpers
sometimes use this. Subtle gap.

**WIT shape:**
```wit
create-sweep-gradient: func(
    cx: f32, cy: f32,
    start-angle: f32, end-angle: f32,
    colors: list<u32>,
    stops: list<f32>,
    tile-mode: tile-mode,
) -> u32;
```

**Host-side:** `skia_safe::shaders::sweep_gradient(...)`.

**Kotlin-side:** two overloads in SkiaTypes.wasi.kt. The
`Gradient`-object overload (line 571) destructures into the same
fields and calls the explicit-params overload (line 578).

### 3. `Image.makeShader(...)`

**Current state:** line 556 throws.
**Why we want it:** image-as-pattern fills (tiled backgrounds,
texture brushes). Less common in our typical UI but enables
arbitrary pattern fills.

**WIT shape:**
```wit
create-image-shader: func(
    image-id: u32,
    tile-x: tile-mode,
    tile-y: tile-mode,
    sampling: sampling-mode,
    matrix: option<matrix-3x3>,
) -> u32;
```

**Host-side:** look up image by id, call
`image.to_shader((tile_x, tile_y), sampling_options, &matrix)`.

### 4. `Bitmap.makeShader(...)` × 2 overloads

**Current state:** `SkiaTypes2.wasi.kt:71, 73` throw.
**Why we want it:** symmetry with `Image.makeShader`. Compose
internals occasionally use Bitmap-shaped APIs; if any path does so
we'd hit this.

**Host-side:** `bitmap.to_shader(...)`. Bitmap on wasi is currently
a thin shim (line ~20 of SkiaTypes2.wasi.kt) — verify the bitmap
has a host-side `skia_safe::Bitmap` backing before wiring.
*Caveat:* if our wasi `Bitmap` is a pure stub with no host
counterpart, we'd need to wire bitmap construction first. Check
this before estimating effort.

### 5. `Shader.makeLinearGradient(Gradient)` + `makeRadialGradient(Gradient)`

**Current state:** lines 567, 569 throw.
**Why we want it:** Compose's `Brush.linearGradient(Gradient(...))`
takes a `Gradient` object directly. We already have stop-list-based
overloads via WIT (`createLinearGradient(...)`); these object
overloads just need to destructure the `Gradient` and forward.

**Kotlin-side:** ~5 lines each — no new WIT, no new host code.
Pure delegation:
```kotlin
fun makeLinearGradient(g: Gradient): Shader {
    val s = g.startPoint; val e = g.endPoint
    return makeLinearGradient(s.x, s.y, e.x, e.y, g.colors.toIntArray(),
        g.stops?.toFloatArray(), g.tileMode)
}
```

### 6. `Shader.makeBlend(blendMode, s1, s2)`

**Current state:** line 580 throws.
**Why we want it:** shader composition. Compose uses this for
certain layered fill effects.

**WIT shape:**
```wit
create-blend-shader: func(
    blend-mode: blend-mode,
    shader1-id: u32,
    shader2-id: u32,
) -> u32;
```

**Host-side:** `skia_safe::shaders::blend(blend_mode, &s1, &s2)`.

### 7. (Out of scope here — covered by task 28)

The abstract `Canvas` 42 stubs.

## Steps

### Step 1 — Inventory the existing image + shader WIT (~30 min)

Open `wit/skiko-gfx.wit` and find the existing
`create-linear-gradient`, `create-radial-gradient`, `drop-shader`,
`draw-image-rect`, `drop-image` definitions. They're the templates
for the new functions. Note the WIT enum types
(`tile-mode`, `sampling-mode`, `blend-mode`) — confirm they cover
what we need or add variants.

### Step 2 — Land Image.makeFromEncoded first (~2 h)

Smallest useful win, separate from shader work.

1. Add WIT verb (and bump `skiko/skiko/wit/skiko-gfx.wit` mirror).
2. Implement in `wart-host/src/canvas_impl.rs` — `from_encoded`
   path. Add a `last_image_id` counter if not already there.
3. Kotlin side: edit `SkiaTypes.wasi.kt:548` to call the new WIT
   import. Regenerate / hand-edit `generated/SkikoUi.kt` and
   `InternalSkikoUi.kt`.
4. Republish skiko klib.
5. Smoke test: load a tiny PNG from wart-app assets, draw it via
   the existing `drawImageRect` path.

### Step 3 — Sweep gradient + Shader.makeBlend (~3 h)

Two more new WIT verbs. Same shape as gradient work in task 11.
Implement, plumb, smoke test by drawing a `Box` with a sweep
fill.

### Step 4 — Image.makeShader + Bitmap.makeShader (~2 h)

One new WIT verb (`create-image-shader`), Bitmap forms forward to
it if our Bitmap is just a thin wrapper around Image. If Bitmap
needs its own host-side type, add it; otherwise skip.

### Step 5 — Gradient-object overload forwarding (~30 min)

Five-line bodies in SkiaTypes.wasi.kt for the linear/radial
`Gradient`-object forms. No WIT changes.

### Step 6 — Full rebuild chain + device verify (~1 h)

1. Republish skiko (`publishWasmWasiPublicationToMavenLocal`).
2. Per `feedback_rebuild_compose_after_skiko`, decide if any
   compose-*-wasi rebuild is needed. Most of these new WIT verbs
   are pure additions, so probably not — but verify via mtime
   check.
3. Rebuild wart-app + repackage cwasm + deploy.
4. Add a small smoke-test card to `RealComposeApp.kt` that
   exercises: an image loaded from encoded bytes, a sweep
   gradient, an image-shader fill, a blend shader. Visual
   verification.

### Step 7 — Commit + update task doc + memory (~30 min)

Single commit per skiko + wart commit per skiko-bindings + a
wart-app commit if we add the smoke card. Update this task doc's
status to `✅ device-verified`. No new memory needed unless
something surprising came up.

## Estimates

| Step | Wall time |
|------|-----------|
| 1. WIT inventory | 30 min |
| 2. Image.makeFromEncoded | 2 h |
| 3. Sweep + Blend shaders | 3 h |
| 4. Image/Bitmap.makeShader | 2 h |
| 5. Gradient-object overloads | 30 min |
| 6. Rebuild chain + device verify | 1 h |
| 7. Commit + doc | 30 min |
| **Total** | **~1 day (9 h focused)** |

## Out of scope

- Path-based gradients (`Path` shape gradients) — not in skiko's
  stub list, would be a new feature.
- HDR images / wide-gamut color — wasi skiko is sRGB-only today.
- Animated image formats (GIF/WebP-anim/APNG) — encoded-image
  decoding here returns a single static frame; animation would
  need a separate path.
- `Picture` recording / `drawPicture` — covered by task 28
  (abstract Canvas).
- `Shader.makeFractalNoise` / `makeTurbulence` — perlin noise
  shaders. Not in skiko's stub list either; add to a follow-up
  if needed.

## Risks

1. **Existing `Bitmap` may have no host backing.** If
   `SkiaTypes2.wasi.kt`'s `Bitmap` is a pure stub without a
   corresponding `state.bitmaps: HashMap<u32, skia_safe::Bitmap>`
   on the host, item 4 needs that infrastructure first. Audit
   before starting step 4; if missing, defer Bitmap.makeShader.

2. **WIT enum coverage.** Verify our `tile-mode`,
   `sampling-mode`, `blend-mode` WIT enums match the variants
   `skia_safe` exposes for the new functions. The blend-mode list
   in particular has 25+ variants; if we only have a few defined,
   `Shader.makeBlend` users will see "unknown variant" errors.

3. **`Gradient` object on wasi.** Check whether `Gradient` is a
   data class in commonMain skiko or only in JVM. If only JVM,
   our wasi sourceset needs a `Gradient` data class definition
   too (likely already there since the stubs reference it).

## Verification checklist

- [ ] `wit/skiko-gfx.wit` and `skiko/skiko/wit/skiko-gfx.wit`
      byte-identical
- [ ] skiko klib republished, mtime newer than compose-*-wasi
      klibs (or compose-*-wasi rebuilt if needed per Step 5
      of `BUILD-wasmWasi.md`)
- [ ] wart-app cwasm builds without errors
- [ ] cold start renders without `Image.X / Shader.X: not
      implemented` exceptions
- [ ] smoke-test card visible on device, all four fills (encoded
      image, sweep gradient, image-shader, blend) render
- [ ] no regressions in existing widgets (text fields, buttons,
      lists, etc.)
