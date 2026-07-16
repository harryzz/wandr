---
name: Android Skia font loading
description: Android FontMgr zero-metrics bug — FIXED by skia-safe 0.99 / Skia m150 (match_family_style now real metrics on device)
type: feedback
originSessionId: ca7f3a70-2c6e-4c65-baae-454dc44933b5
---
## ✅ FIXED by skia-safe 0.99 (Skia m150) — device-proven 2026-07-13
Bumping skia-safe **0.93.1 → 0.99.0** (Skia m144 → m150) fixes the Android zero-metrics bug.
`SkFontMgr_New_Android` is now created with an explicit `SkFontScanner_Make_FreeType()` (via
`C_SkFontMgr_NewSystem`, skia-bindings/src/bindings.cpp), and `match_family_style` returns REAL
metrics on device. Proven with a `--font-probe` on the Pixel (wired into the android `fn main()`
with `android_logger::init_once` → logcat):
```
FontMgr::new(): count_families = 34
  match 'sans-serif': OK  unitsPerEm=2048  glyphs=3362
  match 'serif':      OK  unitsPerEm=2048  glyphs=2409
  match 'monospace':  OK  unitsPerEm=2048  glyphs=895
```
So `get_typeface` (canvas_impl.rs) no longer needs the `#[cfg(not(target_os="android"))]` gate on
its `match_family_style` block — by-name resolution is now UNIFIED desktop+device (the units_per_em
> 0 && count_glyphs > 0 guard falls through to TTF-path candidates defensively / for Compose family
names Android doesn't expose, e.g. `'Roboto'`/`'Noto Sans'` → None; use generics sans-serif/serif/monospace).
Note: `SkFontMgr_New_AndroidNDK` is NOT exposed by skia-safe 0.99 — `NewSystem()` uses the parser-based
`SkFontMgr_New_Android(nullptr, FreeType)`, and that's sufficient. See `[[reference_desktop_font_resolve_by_name]]`.

---
## Historical (skia-safe 0.93.1 / m144) — the bug this replaced
`FontMgr::default().match_family_style()` on Android with the m144 Skia/NDK build returns typefaces with zero glyph advance widths. TextBlob is created successfully (`blob=true`) but `bounds()` returns `(0,0,0,0)` so all text is invisible.

**Fix:** Read the TTF bytes with `std::fs::read(path)` then create via `fm.new_from_data(&bytes, None)`.

**Why:** The system FontMgr on Android doesn't properly parse font metrics for these typefaces — loading the raw bytes through `new_from_data` gives Skia the full TTF data it needs for real metrics.

**How to apply:** Always use `new_from_data` for text on Android. Cache the loaded `Typeface` in `SkiaRenderer` (two fields: `typeface_regular` and `typeface_bold`) — loading from disk takes ~40ms, which kills frame rate if repeated every frame.

Candidate paths in order: `/system/fonts/Roboto-Regular.ttf`, `/system/fonts/NotoSans-Regular.ttf`, `/system/fonts/DroidSans.ttf` (regular); `/system/fonts/Roboto-Bold.ttf`, `/system/fonts/NotoSans-Bold.ttf`, `/system/fonts/DroidSans-Bold.ttf` (bold).

Also: `Typeface::from_file` and `Typeface::default()` do NOT exist in skia-safe 0.93.1. Use `FontMgr::new_from_data` and `fm.legacy_make_typeface` as fallback.
