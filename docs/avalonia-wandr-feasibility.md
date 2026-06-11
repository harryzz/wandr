# Avalonia on wandr — feasibility memo

> Written 2026-06-11, right after task 100 (Slint) shipped, as the "does the
> pattern generalize?" record. Source-grounded: Avalonia master inventoried at
> /tmp/avalonia; .NET-component toolchain per
> `docs/wasm-component-language-support.md` + componentize-dotnet upstream.
> Status: **analysis only — no spike run.**

## Verdict

**Yes — Avalonia fits the same mold as Slint, and `my:skiko-gfx` already
covers its drawing surface.** It is the same architectural shape that made
the Slint port cheap: a UI framework whose primary backend is Skia, with a
**pluggable rendering abstraction** and **self-shaped glyph-level text** —
exactly what `docs/skia-wit-mapping.md` calls "any UI library with a Skia
backend." The difference is the *runtime*: C#/.NET instead of Rust, which
moves the risk from the renderer (small, well-understood after task 100) to
the **toolchain and footprint** (componentize-dotnet is preview; a NativeAOT
.NET runtime + Avalonia in one wasm component will be several times the
8.7 MB Slint guest). Effort estimate: **2–4 weeks** vs Slint's ~2 days,
almost all of it toolchain bring-up and C#-side plumbing, not WIT work.

## Why the pattern transfers (the three seams)

Avalonia's platform abstraction is *more* formalized than Slint's:

1. **Rendering — `IPlatformRenderInterface` + `IDrawingContextImpl`**
   (`src/Avalonia.Base/Platform/`). The draw-op surface is a close cousin of
   Slint's ItemRenderer and maps almost 1:1 onto the canvas WIT (table
   below). Proof a non-Skia implementation is viable ships in-tree:
   `HeadlessPlatformRenderInterface` (618 lines, zero Skia) — the natural
   template for a `WandrPlatformRenderInterface`, the way `FemtoVGRenderer`
   was the template for slint-wandr.

2. **Text — `ITextShaperImpl` / `IGlyphTypefaceImpl` / `IFontManagerImpl`.**
   Avalonia shapes its own text (HarfBuzz via the standalone
   `src/HarfBuzz/Avalonia.HarfBuzz` package — pluggable, NOT welded to the
   Skia backend) and hands the renderer **positioned glyph runs**
   (`DrawGlyphRun`). That is precisely the Slint/parley model, so the
   task-100 glyph verbs (`create-typeface` from the guest's font bytes +
   `draw-glyphs`) fit unchanged — same glyph-id-consistency argument, same
   `/system-fonts` preopen for font bytes, same
   `GenericFamily`-style fallback wiring on the C# side.

3. **Platform — windowing/input/threading.** Avalonia's Browser backend
   already runs the whole framework **single-threaded on wasm** (browser
   wasm via Mono + Emscripten) — the existence proof that nothing in
   Avalonia's core needs threads or a real windowing system. A wandr
   platform = the dioxus/slint wiring: `renderer` export events →
   `RawPointerEventArgs`/`RawKeyEventArgs`, `render-frame` → the composition
   tick, `frame-pacing` ← Avalonia's dirty/animation state, IME via
   `TextInputMethodClient` → `notify-editor-attached/-detached` (the ESC
   hide convention from task 100 applies as-is).

## Draw-op mapping (inventoried from IDrawingContextImpl)

| Avalonia op | WIT verb | Notes |
|---|---|---|
| Clear, DrawLine, DrawRectangle (rrect+BoxShadows), DrawEllipse | `clear`, `draw-line`, `draw-rrect`/`draw-shadow-rrect`, `draw-oval` | ✅; Avalonia BoxShadows ride the task-100 shadow verb |
| DrawGeometry / Push-/PopGeometryClip | `draw-path` / `clip-path` (SVG strings) | ✅ — `IStreamGeometryImpl` records path segments; serialize to SVG like lyon |
| DrawGlyphRun | `draw-glyphs` + `create-typeface` | ✅ task-100 batch |
| DrawBitmap, LoadBitmap*, IWriteableBitmapImpl | `create-image(-from-encoded)` + `draw-image-rect` | ✅ decode guest-side (ImageSharp or host `create-image-from-encoded`) |
| PushClip / PopClip | `clip-rect`/`clip-rrect` + save/restore | ✅ guest-side clip tracking (femtovg pattern) |
| PushOpacity / PopOpacity, PushLayer / PopLayer | `save-layer` (+ the extra-restore counter trick from slint-wandr) | ✅ |
| CreateOffscreenRenderTarget / CreateRenderTargetBitmap | `create-bitmap-canvas` + `bc-*` + `bitmap-canvas-snapshot` | ✅ |
| Brushes: solid / linear / radial / conic | paint color / `create-*-gradient` shader ids | ✅ |
| TileBrush / VisualBrush | render to bitmap-canvas → `create-image-shader` (repeat tiling) | ✅ (this is what image tiling needs anyway) |
| PushOpacityMask | — | 🚫 gap (saveLayer with mask paint); rare — defer like Slint's |
| PushEffect (BlurEffect / DropShadowEffect on arbitrary content) | — | 🚫 gap: needs an image-filter-on-layer verb (the one Skia area the WIT still lacks); DropShadow on *shapes* maps to `draw-shadow-rrect` |
| DrawRegion / dirty-region composition | n/a | host repaints full frames; fine at wandr's frame model |

Net: **zero new WIT verbs required** for a first port (effects/opacity-mask
deferred, same as Slint's deferred list).

## The hard parts (all runtime/toolchain, not rendering)

1. **Toolchain: componentize-dotnet (preview).** NativeAOT-LLVM compiles C#
   → native wasm (its own GC inside linear memory — like Rust, NO wasm-gc
   needed, so our wasmtime AOT flags and the Kotlin-style adapter pain don't
   apply). WIT imports/exports are generated (wit-bindgen-dotnet) — our
   `renderer`/`frame-pacing` world is expressible. Risks: preview-grade
   (~0.6–0.7), historically **Windows-only NativeAOT-LLVM builds** (would
   mean building guests on the Windows side of this WSL2 box — verify
   current state when attempting), and nobody has pushed a dependency graph
   as big as Avalonia through it. This is THE derisking spike.
2. **HarfBuzz native dependency.** `Avalonia.HarfBuzz` P/Invokes
   HarfBuzzSharp's native lib. harfbuzz is plain C and compiles for
   wasm32-wasi (the browser backend already links it into wasm), but
   getting NativeAOT-LLVM to statically link a wasi-built harfbuzz is
   plumbing that needs proving. Fallback: a managed shaper is NOT available
   — this is a hard prerequisite, unlike Slint where parley is pure Rust.
3. **Footprint.** Slint guest = 8.7 MB wasm. Avalonia + .NET NativeAOT
   runtime + HarfBuzz + themes will plausibly land 30–80 MB → bigger AOT
   cwasm, slower install-time precompile, bigger per-app working set
   (budget ~180 MB/app, see app-lifecycle memory). Measurable only by the
   spike.
4. **API stability.** The platform interfaces are `[NotClientImplementable]`
   /unstable-annotated — Avalonia reserves the right to break backend
   authors between minors (same deal as `i-slint-*`: pin exactly, re-check
   on bump). SkiaSharp/HarfBuzzSharp props must be kept OUT of the custom
   backend's csproj (core `Avalonia.Base` doesn't reference them).
5. **XAML + trimming.** Avalonia's compiled XAML is NativeAOT-friendly
   (officially supported on desktop NativeAOT); reflection-heavy app code
   would need the usual trimmer annotations.

## Recommended spike order (if/when wanted)

1. **Hello-component:** componentize-dotnet → a C# guest exporting our
   `renderer` world, drawing one `draw-rect` — proves toolchain + WIT +
   on-device AOT in one shot (this is the survey's "drop-in next" claim,
   now with a concrete world to target). Biggest unknown, smallest test.
2. **harfbuzz-wasi link test** inside that guest.
3. **Headless-template backend:** `WandrPlatformRenderInterface` cloned from
   HeadlessPlatformRenderInterface, draws through the WIT.
4. Then the platform/input/IME wiring is task-100 muscle memory.

## How it slots into the guest-UI lineup

| | Compose (Kotlin) | dioxus-canvas (Rust) | Slint (Rust) | **Avalonia (C#)** |
|---|---|---|---|---|
| Status | shipping | shipping (production) | shipped, eval (task 100) | **analysis only** |
| Text model | host-shaped (paragraph iface) | host-shaped (text blobs) | guest-shaped (parley → draw-glyphs) | guest-shaped (HarfBuzz → draw-glyphs) |
| Runtime | WasmGC + adapter pin | native wasm | native wasm | NativeAOT (own GC, no wasm-gc) |
| WIT gaps | — | — | closed 2026-06-11 | **none for v1** (effects deferred) |
| Main risk | (paid already) | — | upstream pin (1.17-dev) | toolchain maturity + footprint |

Avalonia would be the proof that the skia-wit contract is genuinely
language-agnostic — the first non-Rust, non-Kotlin consumer. It's also the
richest widget set of the four. But it should wait for a concrete need (a
C#-team app, or wanting Avalonia's control library); the spike order above
turns it into an estimate in ~a day of toolchain work.
