---
name: reference_desktop_font_resolve_by_name
description: "Skia's system FontMgr DOES resolve OS-installed fonts by name with real metrics on DESKTOP (Linux verified: 263 families) — the CLAUDE.md zero-metrics ban is Android-ONLY. Host get_typeface now uses match_family_style on desktop; use --font-probe to verify a platform."
metadata:
  node_type: memory
  type: reference
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

Verified 2026-07-12 via `wasm-android-host --font-probe` (a new diagnostic that tests
Skia's system FontMgr): on **Linux desktop**, `FontMgr::new()`/`FontMgr::default()` have
**263 families** and `match_family_style` resolves by NAME with **real metrics**
(DejaVu Sans unitsPerEm=2048 glyphs=6253; "sans-serif"→Noto Sans; "Arial"→Liberation Sans;
"monospace"→DejaVu Sans Mono; unresolvable→None). skia-safe 0.93.1.

So the CLAUDE.md rule "FontMgr::default().match_family_style() returns zero-metrics
typefaces" is **Android-ONLY** — the Pixel's system FontMgr is broken; desktop
(fontconfig/DirectWrite/CoreText via the skia prebuilt) is healthy.

Change made: `canvas_impl.rs get_typeface()` now, on `#[cfg(not(target_os="android"))]`
and a non-path family, tries `FontMgr::new().match_family_style(family, style)` FIRST
(guarded by unitsPerEm>0 && glyphs>0), before the file-path candidates. This makes
OS-installed fonts resolvable BY NAME on desktop — the basis for "app prerequisite:
install font X" (Windows install/API, `fc-cache` on Linux, Font Book on Mac). Windows/Mac
not yet probed but should work (platform font managers); run --font-probe there to confirm.

IMPORTANT nuance: **OpenSwiftUI/Swift guests DON'T use get_typeface** — they read font
bytes from the `/system-fonts` preopen and call `typeface-from-bytes`/`draw_glyphs`
(guest owns the font). The family-name path (get_typeface) is used by family-passing
guests (Compose/Slint) via wasi_canvas draw with `s.family`. So the resolve-by-name helps
those; for OpenSwiftUI `Image(systemName:)`, either (a) read the icon font from
/system-fonts (or an app `[[mounts]]`) via typeface-from-bytes, or (b) route the icon
through the family-name draw so the host resolves the OS-installed font. See
[[reference_observableobject_wasm_exclusivity]] and the 2048 port thread.
