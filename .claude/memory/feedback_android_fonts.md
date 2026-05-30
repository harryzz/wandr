---
name: Android Skia font loading
description: FontMgr::default().match_family_style() returns zero-metrics typefaces on Android — must load TTF bytes directly
type: feedback
originSessionId: ca7f3a70-2c6e-4c65-baae-454dc44933b5
---
`FontMgr::default().match_family_style()` on Android with this Skia/NDK build returns typefaces with zero glyph advance widths. TextBlob is created successfully (`blob=true`) but `bounds()` returns `(0,0,0,0)` so all text is invisible.

**Fix:** Read the TTF bytes with `std::fs::read(path)` then create via `fm.new_from_data(&bytes, None)`.

**Why:** The system FontMgr on Android doesn't properly parse font metrics for these typefaces — loading the raw bytes through `new_from_data` gives Skia the full TTF data it needs for real metrics.

**How to apply:** Always use `new_from_data` for text on Android. Cache the loaded `Typeface` in `SkiaRenderer` (two fields: `typeface_regular` and `typeface_bold`) — loading from disk takes ~40ms, which kills frame rate if repeated every frame.

Candidate paths in order: `/system/fonts/Roboto-Regular.ttf`, `/system/fonts/NotoSans-Regular.ttf`, `/system/fonts/DroidSans.ttf` (regular); `/system/fonts/Roboto-Bold.ttf`, `/system/fonts/NotoSans-Bold.ttf`, `/system/fonts/DroidSans-Bold.ttf` (bold).

Also: `Typeface::from_file` and `Typeface::default()` do NOT exist in skia-safe 0.93.1. Use `FontMgr::new_from_data` and `fm.legacy_make_typeface` as fallback.
