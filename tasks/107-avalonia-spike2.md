# Task 107 — AvaloniaUI spike #2: harfbuzz-wasi + Avalonia-proper on wasi

Follow-up to task 106 (spike #1 GO). Memo: `docs/avalonia-wandr-feasibility.md`.
Two questions left between us and a real port:

- **A. Text:** can harfbuzz link as a wasm32-wasi native dep inside a
  componentize-dotnet guest, shape real text, and draw through
  `wasi:canvas/glyphs@0.0.2`? (The memo's "hard prereq, no managed shaper
  exists".)
- **B. Framework:** does Avalonia-proper (12.0.4) compile under
  NativeAOT-LLVM → wasi and *initialize* — locator, Dispatcher, compositor,
  layout — using the in-tree headless backend (the zero-Skia
  `HeadlessPlatformRenderInterface` this port would clone)?

## Part A — text-guest: ✅ PASSED (2026-06-12)

`repros/avalonia-spike2/text-guest/`:

- harfbuzz **14.2.1** single-file amalgamation (`src/harfbuzz.cc`) compiles
  clean with the toolchain's own wasi-sdk-24 clang++ (`-O2 -DHB_NO_MT
  -fno-exceptions -fno-rtti`, 1m08s) → `native/libharfbuzz.a` (530 `hb_*`
  symbols).
- Linked via stock NativeAOT items: `<DirectPInvoke Include="harfbuzz"/>`
  + `<NativeLibrary Include="native/libharfbuzz.a"/>` (componentize-dotnet
  needs no extra plumbing; the link step is `$(WASI_SDK_PATH)/bin/clang`).
- C# guest shapes "Avalonia spike #2 — harfbuzz: AV fi ffl" with font
  scale = upem: 39 chars → **36 glyphs (fi + ffl ligatures formed)** —
  real OpenType shaping inside the component.
- Draws via `glyphs.typeface.from-bytes` (same `/system-fonts/
  NotoSans-Regular.ttf` bytes the shaper used) + `draw-glyphs`; em size
  derived per-frame so the line spans 0.8 × surface width. Rendered
  correctly on the desktop host at 500x1000; `text_guest.wasm` Release =
  **4.2 MB** (vs 2.57 bare), render_frame steady ~3 ms (first frame 29 ms —
  shaping + typeface decode).

This is the Avalonia text model in miniature: guest-side shaper emits
positioned glyph ids against the guest's own font bytes; host rasterizes
from those exact bytes.

## Part B — avalonia-headless: in progress

`repros/avalonia-spike2/avalonia-headless/`: console wasi component
referencing **Avalonia 12.0.4 + Avalonia.Headless + Themes.Fluent**,
code-only App (no XAML), `UseHeadless(UseHeadlessDrawing=true)` +
`SetupWithoutStarting()`, build a Window+StackPanel+Button, `RunJobs()`,
print bounds, `ForceRenderTimerTick()`.

Findings so far:

1. **It compiles.** Full Avalonia through NativeAOT-LLVM → 42.7 MB wasm
   (6m28s, zero errors) — the memo's 30–80 MB projection lands at the low
   end.
2. **It runs** up to `MediaContext..ctor`, then
   `MissingMethodException: TimeSpan.FromMilliseconds(Int64)` — corelib
   version skew: the template's pinned ilcompiler
   (`10.0.0-alpha.1.25162.1`, 2025-03) predates the .NET 9+ overload that
   Avalonia's net10.0 assembly calls.
3. Fix attempt: bump `runtime.linux-x64.microsoft.dotnet.ilcompiler.llvm`
   to `10.0.0-preview.2.25509.1` (newest on dotnet-experimental,
   2025-10) — result pending.
4. 12.0.4's headless `RenderTimer` is DispatcherTimer-based
   (`RunsInBackground => false`) — single-threaded, wasi-safe (the
   master-branch `ShouldRenderOnUIThread` option doesn't exist in 12.0.4
   and isn't needed).
5. `UseHeadless` force-chains `UseHarfBuzz()`: HarfBuzzSharp's
   `DllImport("libHarfBuzzSharp")` resolves against the same
   `libharfbuzz.a` via `<DirectPInvoke Include="libHarfBuzzSharp"/>` —
   if layout completes, Avalonia's REAL text stack ran against our
   archive (binding is harfbuzz 8.3 API; 14.2.1 is an ABI superset).

## Part B result: ✅ Avalonia 11.3.17 runs on wasi

- Avalonia **12.0.4** compiles (42.7 MB) but its net10.0 assets call
  `TimeSpan.FromMilliseconds(long)`, which the pinned ILC alpha corelib
  lacks; the newest experimental ILC (`preview.2.25509.1`) builds a wasm
  that traps at module init ("uninitialized element") against
  componentize-dotnet 0.7.0's link. **11.3.17 + the pinned alpha ILC is
  the working pair** — init + Fluent layout + frame tick verified under
  wasmtime (probe: `repros/avalonia-spike2/avalonia-headless`).

## Part C (added): ✅ full controls demo on the desktop host

`repros/avalonia-spike2/avalonia-demo` — Avalonia 11.3.17 + FluentTheme
dark as a wandr reactor guest (41.3 MB wasm), USER-INTERACTIVE on the
desktop host: header text, gradient bar, Button (click counter),
CheckBoxes, ToggleSwitch, RadioButtons, Slider→ProgressBar wiring,
TextBox (typing works), ListBox w/ selection + scrollbar. ~2-4 ms/frame
steady, ~100 ms first frame, ~16 s JIT load.

The platform layer (`Platform/`, ~1100 lines — the spike-3 work, done):
- `WandrRenderInterface : IPlatformRenderInterface(+Context)` — needs
  csproj `<AvaloniaAccessUnstablePrivateApis>true` (lib-not-ref asm).
- `WandrDrawingContext : IDrawingContextImpl` → wasi:canvas; absolute
  Transform synced by concat'ing `pending × currentⁱⁿᵛ` deltas; clips/
  opacity via save/clip / save-layer; brushes → solid color + linear/
  radial/sweep gradient shaders; BoxShadows → mask-blur rrects.
- `WandrGlyphTypeface/WandrFontManager/WandrTextShaper` — Avalonia.Skia
  11.3.17 text stack with SKTypeface replaced by HarfBuzzSharp Face/Font
  over /system-fonts bytes; same bytes feed `glyphs.typeface.from-bytes`.
- Geometry = SVG path-data strings (draw-path/clip-path direct);
  combined geometry via host `graphics.combine-paths`.
- Input: pointer/key exports → `HeadlessWindowExtensions`
  (MouseDown/KeyPressQwerty/KeyTextInput); W3C code → PhysicalKey.

**Three hard-won integration lessons (the actual spike findings):**
1. `IRenderTarget2` is REQUIRED: a plain IRenderTarget makes the
   compositor demand `CreateLayer` (intermediate layer) and silently
   swallows the NotSupportedException → context created, zero draws.
   Report `IsSuitableForDirectRendering=true`.
2. Avalonia renders INCREMENTALLY (dirty rects) and assumes a retained
   target; the wandr frame buffer arrives cleared → partial frames
   (mouse-over turned the screen black: only the hovered control got
   drawn). Fix: render into a persistent `graphics.new-offscreen` canvas
   and `snapshot()`-blit it to the frame buffer each present — honest
   retained semantics, full frame every present.
3. .NET thread pool never runs on single-threaded wasi (probe: queued
   item never executed) — but Avalonia's commit throttling still works
   because its `ContinueWith(..., ExecuteSynchronously)` inlines on the
   completing (UI) thread. Don't rely on the pool in guest code.

Not implemented (loud NotSupportedException, none hit by the demo):
bitmaps, offscreen render-target bitmaps, regions, opacity masks,
text-as-geometry. Popup-window controls (ComboBox, tooltips, menus)
deliberately excluded — popups create separate top-levels that would
need surface/role wiring (later, with the arbiter model).

## Exit criteria — ALL MET 2026-06-13

A: shaped+ligated text from C# ✅. B: Avalonia runs on wasi ✅ (11.3.17).
C: interactive Fluent controls demo on the desktop host ✅ (verified with
synthetic XTEST input: clicks counted, checkbox toggled, slider dragged
40→80 with ProgressBar tracking, "Hi wandr" typed, list selection moved).

## Part D (added): ✅ ON DEVICE — Pixel 2 XL

Packaged as `apps/user/wandr.avalonia.demo` (package.toml + components/
ui.wasm) and installed the normal way — **no dev-machine aarch64 AOT
needed; the on-device host precompiles**:

```
adb push apps/user/wandr.avalonia.demo /data/local/tmp/wandr.avalonia.demo.wandrpkg
adb shell "su -c 'WANDR_APPS_ROOT=/data/local/tmp/wandr-apps \
  LD_LIBRARY_PATH=/data/local/tmp /data/local/tmp/wandr-host --install \
  /data/local/tmp/wandr.avalonia.demo.wandrpkg'"
adb shell "su -c '/data/local/tmp/wandr-arbiter preload wandr.avalonia.demo'"
adb shell "su -c '/data/local/tmp/wandr-arbiter launch  wandr.avalonia.demo'"
```

- **On-device precompile of the 41 MB component → 88 MB ui.cwasm in 25 s**
  (the real unknown — fine on the Pixel 2 XL; no OOM).
- Renders correctly on the real panel under the live wandr status bar
  (full GPU/EGL path, ~174 MB RSS running). Same guest binary as desktop.
- **Font fix (no-hardcode):** WandrFontManager now DISCOVERS the base sans
  by read-probing `/system-fonts` in preference order
  (NotoSans-Regular→Roboto-Regular→DroidSans) instead of the desktop-only
  `NotoSans-Regular.ttf`. Device has Roboto-Regular (no NotoSans-Regular,
  no Roboto-Bold → bold falls back to regular). Desktop unchanged.
- **Touch:** `adb shell input` is gone under --no-art ("Can't find service:
  input" — it's an ART command); on-device input is wandr-inputflinger
  (evdev), the same path every wandr app uses. The full pointer/key →
  Avalonia chain was already proven interactively on desktop; physical tap
  on the panel is the remaining user-side confirmation.

All four parts (A harfbuzz-text, B Avalonia-on-wasi, C desktop controls
demo, D on-device) done 2026-06-13. Avalonia is a working wandr guest
language end-to-end.

### Part D follow-up: HiDPI scaling (device UI was unreadably small)

First device render was correct but TINY — the headless window impl
reports `RenderScaling = 1`, so Avalonia laid out 1 logical-px = 1
physical-px on the 1440-wide panel. Fix (no hardcoded multiplier):
- Import `wandr:ui-shell/metrics@0.1.0` (subset copy in wit/deps); query
  `get-density()` (dpi/160 — the scale factor Slint uses) once.
- Avalonia still renders at RenderScaling 1 (the headless impl can't
  report otherwise), so map logical→physical by a **base `scale(density)`
  on the retained canvas each frame**, size the window in **logical units**
  (physical/density), and convert incoming **pointer coords by 1/density**.
- Desktop host reports density 2.0; device reports its panel density.
  Verified readable + correctly scaled on the Pixel 2 XL; touch confirmed
  (button counter incremented via injected evdev taps — pointer density
  conversion correct).

### Device DPI scaling + miniature-copy artifact — RESOLVED

Two device-only rendering bugs, fixed together:
- **Unreadably small UI:** the headless window impl reports
  `RenderScaling = 1`, so Avalonia laid out 1 logical-px = 1 physical-px on
  the 1440-wide panel. Fix (no hardcoded multiplier): import
  `wandr:ui-shell/metrics`, query `get-density()` (dpi/160) once, scale the
  retained canvas by density per frame (FrameBridge BeginFrame Save+Scale),
  size the window in logical units, convert pointer coords by 1/density.
- **Miniature copy of the whole UI in the top-left (appeared on
  INTERACTION):** the true cause, found by logging whether each
  `CreateDrawingContext` happened inside the on-frame bracket — input
  events (`on-pointer`/`on-key`) make Avalonia render SYNCHRONOUSLY,
  outside `BeginFrame`/`EndFrame`, where the density base scale isn't
  active. Those renders drew the whole UI UNSCALED (logical-size, top-left)
  into the retained canvas. **Fix:** gate the drawing canvas —
  `FrameBridge.CurrentCanvas => InFrame ? _retained : null` — so
  out-of-frame renders no-op; all drawing happens in the scaled on-frame
  pass. Input still updates state; the next frame's full repaint
  (`Window.InvalidateVisual()` + `PreviousFrameIsRetained = false`) shows
  it. NO extra canvas save/restore → no aarch64 crash. Verified on device:
  clean after dragging through the controls, stable 75s+.

  (Earlier mis-diagnosis: thought it was a stale burn-in fixable by full
  redraw alone — that helped at rest but the mini returned on interaction,
  because the unscaled render is re-issued every input event. The gate is
  what actually stops it.)

**Approaches that FAILED on device (kept here as a map):** isolating the
density base per drawing-context (Save/Scale per context, Restore on
Dispose) removed the mini on desktop but **SIGSEGV'd on the device's
aarch64 AOT** — the compositor's push/pop aren't perfectly balanced and the
extra canvas Restore underflowed; depth-guarding the saves didn't help. A
logical-sized offscreen + upscale-blit also crashed (masked .NET exception,
crash reporter faulted in `CrashInfo.WriteChars`). The winning fix adds NO
extra canvas save/restore — it just forces full redraws on the stable
frame-level base, so it sidesteps the aarch64 fault entirely. Verified on
the Pixel 2 XL: clean full-size UI, no mini, no crash, stable 77s+.
Desktop note: full redraw also fine there (crisp, no mini).

### Part F (added): ✅ IME / soft keyboard — device-verified

The TextBox was dead on-device (no hardware keyboard). Wired the soft
keyboard the same way slint-wandr does — import the EXISTING
`wandr:ui-shell/ime` (subset copy in wit/deps, like metrics; NOT a new
interface) and call `notify-editor-attached`/`-detached` on editor focus;
typed text returns through the already-wired `key-handler` export.

Avalonia resolves its IME via `TopLevel → PlatformImpl.TryGetFeature<
ITextInputMethodImpl>()`, but `HeadlessWindowImpl` returns null there and
is `internal` (can't subclass to provide one). So instead of Avalonia's
text-input-method plumbing, `WandrIme.Sync()` **polls the focused element
each on-frame** (`FocusManager.GetFocusedElement() as TextBox`) and
reconciles: on a new TextBox focus → `notify-editor-attached(input-type,
hint=Watermark, text, char-offset selection)`; on blur → `-detached`.
Attach once on focus, never per-keystroke (slint-wandr does the same —
re-attaching churns the overlay). ESC = the keyboard's hide button
(task-47 convention) → `FocusManager.ClearFocus()` → detach.

Device-verified (Pixel 2 XL): tap TextBox → `[editor:text]` in the arbiter
+ `wandr.ime.keyboard` goes `[fg]`; typed "hia" landed in the field; tap
the button → keyboard hid, editor detached, and the tap counted (Click me:
1). Files: `Platform/WandrIme.cs`, the ime import in `wit/world.wit` +
`wit/deps/wandr-ui-shell/ui-shell.wit`, focus-sync call + ESC handling in
`Exports.cs`. No crash.

### Part G (added): ✅ reusable library extracted

Split the demo into a reusable lib + a thin app:
- **`dotnet/avalonia-wandr/`** — the reusable C#/.NET guest UI adapter (the
  .NET peer of `crates/slint-wandr`; new top-level `dotnet/` documented in
  `docs/repository-layout.md`). Contains `src/` (render backend, text,
  input, IME, the runtime + WIT exports + `Host.Configure` hook), `wit/`
  (fixed world `wandr:avalonia-guest`), `native/libharfbuzz.a`, and
  `avalonia-wandr.props`. README explains the model.
- **`apps/user/wandr.avalonia.demo/`** — now holds the app SOURCE too
  (`DemoApp.cs` = UI, `AppInit.cs` = `[ModuleInitializer]`→`Host.Configure`,
  `avalonia-demo.csproj` = 3 lines importing the props) alongside
  `package.toml` + `components/`.

**Distribution = SHARED SOURCE, not a .dll.** componentize-dotnet generates
WIT bindings per-project against the world name, so the generated
`GuestWorld` namespace must live in the same assembly as the lib code using
it — a precompiled .dll can't share those types. So the lib compiles into
the consumer via the .props, against a fixed world name. An app provides
only its `Application` + root `Window`. Rebuilt + verified from the new
`apps/user/` location (build OK; device run unchanged — same WIT surface,
same 41 MB wasm).

### KNOWN ISSUE: high idle CPU (~60%)

The render loop repaints EVERY frame (`InvalidateVisual` +
`ForceRenderTimerTick`, no dirty/on-demand gating) → continuous rendering
even when idle. Full redraw is currently load-bearing (suppresses the
unscaled input-render artifact). Fix = on-demand rendering (render only on
Avalonia dirty / frame-pacing, like the Rust guests —
`reference_on_demand_rendering`). NOT investigated yet (user flagged
2026-06-13). Noted in `dotnet/avalonia-wandr/README.md`.

### Known limitation: transient-overlay ghosts

With the retained-offscreen + incremental compositor, a transient overlay
(tooltip, drag adorner, popup) that later closes leaves residue — the main
window's dirty rects don't cover the vacated region. Surfaced by the
synthetic unlock-swipe (a full-screen evdev drag bleeding INTO the app
across all controls); a real finger-swipe on the keyguard (separate
top-level) wouldn't. Attempted fix (clear offscreen + force full redraw
each frame) cleared ghosts on DESKTOP but produced BLACK frames on DEVICE
(forced full redraw is unreliable under the device's frame timing — the
snapshot races the not-yet-complete render) → **reverted to retained +
incremental**. Same class as the popups-out-of-scope limitation; a proper
fix needs per-overlay region invalidation or a reliable full-redraw
barrier. Steady-state (no transient overlays) renders clean.
