---
name: Paint alpha pipeline must combine color.alpha × paint.alpha (skia's model is ONE alpha)
description: Why witAttrs() in WasiCanvas multiplies the alpha-from-color and the alpha-from-paint into a single effective byte, and what breaks when it doesn't.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
SkPaint has ONE alpha — the high byte of its SkColor. `paint.setAlpha(a)` and `paint.setColor(c)` aren't independent: setColor includes alpha, setAlpha rewrites only the alpha byte of the current color.

Our `org.jetbrains.skia.Paint` shim splits them into two fields (`color: Int` ARGB and `alpha: Int` 0..255). Compose's wasi compose-ui-graphics calls into this shim either way:

- Sometimes via `paint.color = someColor.copy(alpha = 0.12f).toArgb()` — alpha encoded in the top byte of `color`, paint.alpha left at default 255.
- Sometimes via `paint.setAlpha(...)` — paint.alpha changes, color.alpha stays at 255.
- Sometimes both (Compose's `MaterialIndicationInstance` stateLayer combines them).

Bug pattern (verified 2026-05-12): the previous `Paint.witAttrs()` sent `color.toUInt()` and `alpha.toUByte()` separately. Host's `make_paint` then did `set_argb(...)` (which writes ALL four bytes from `color`) followed by `set_alpha(attrs.alpha)` (which OVERWRITES the alpha byte). Result: when Compose set color-alpha to ~30 (12% opacity for a state-layer) but left paint.alpha=255, the host rendered fully opaque.

Visible symptoms before the fix:
- Material3 Button stuck in "dark purple pressed visual" after release — the state-layer overlay was rendering opaque instead of fading. (Looked like the press hadn't released even though `onClick` had fired and `count` had incremented.)
- Material3 Switch had a heavy black stroke — the track's outline color was a translucent black that got slammed to opaque.
- Material3 Button with `outlinedButtonColors()` rendered as a solid black pill — its container `Color.Transparent` (alpha=0) was being overridden to opaque, so the "transparent" fill rendered as opaque-with-RGB-=-0 = solid black.

Fix in `org/jetbrains/skiko/WasiCanvas.kt::witAttrs()`:

```kotlin
val colorAlpha = (color ushr 24) and 0xFF
val effectiveAlpha = ((colorAlpha * alpha) / 255).coerceIn(0, 255)
val packedColor = (effectiveAlpha shl 24) or (color and 0x00FFFFFF)
return WitCanvas.PaintAttrs(color = packedColor.toUInt(), alpha = effectiveAlpha.toUByte(), ...)
```

Both the packed color AND the alpha field carry the same combined value, so whichever path the host actually consults (set_argb's color → alpha, or the separate set_alpha call) ends up with the same number. The host's set_argb-then-set_alpha sequence is now idempotent for alpha.

**How to apply:**
- When you add new paint properties (e.g. stroke-width modulator, dash pattern with alpha…), follow the same rule: compute the effective value Kotlin-side from BOTH the explicit field and any modulator that Compose might multiply in.
- If a Compose rendering becomes "too transparent" or "missing entirely" after a paint refactor, suspect that you're now correctly applying an alpha that was previously being overridden — verify by checking what color the upstream Composable sets. Often a low-alpha or transparent fill IS the intended behavior (e.g. `Modifier.background(Color.Transparent)` should produce no fill).
- Do NOT try to detect "color.alpha is < 255 → ignore paint.alpha" or similar heuristics. The skia rule is just multiplication; honor it always.
