---
name: reference_avalonia_wandr
description: "Avalonia on wandr WORKS END-TO-END incl. ON DEVICE (tasks 106-107, 2026-06-13): Fluent controls demo runs on the Pixel 2 XL (apps/user/wandr.avalonia.demo; 41MB wasm→88MB on-device cwasm, precompile 25s) AND interactive on desktop host; pin Avalonia 11.3.17 (12.x = corelib skew); gotchas: IRenderTarget2 required, retained-offscreen+snapshot blit, font manager must DISCOVER (Roboto on device, Noto on desktop), thread pool dead on wasi; memo = docs/avalonia-wandr-feasibility.md"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 66372abf-b0cb-483c-b52e-5b3445aa9260
---

**Avalonia on wandr — analyzed 2026-06-11 (no spike run). Full memo:
`docs/avalonia-wandr-feasibility.md`; source inventory at /tmp/avalonia.**

- Same shape as Slint: Skia-backed UI framework with a pluggable render
  abstraction (`IPlatformRenderInterface`/`IDrawingContextImpl`; in-tree
  proof = 618-line `HeadlessPlatformRenderInterface`, zero Skia) and
  GUEST-shaped glyph-level text (`ITextShaperImpl` via the standalone
  `Avalonia.HarfBuzz` package → `DrawGlyphRun`) → the task-100
  `create-typeface`/`draw-glyphs` verbs fit unchanged.
- **Zero new WIT verbs for a v1 port** — full draw-op mapping in the memo;
  only deferred gaps are PushOpacityMask + PushEffect (blur-on-content
  needs the image-filter-on-layer verb the WIT still lacks).
- Risk lives in the RUNTIME, not rendering: componentize-dotnet
  (NativeAOT-LLVM, preview ~0.6–0.7, historically Windows-only builds —
  verify), harfbuzz must link as wasm32-wasi native dep (hard prereq, no
  managed shaper exists), footprint ~30–80 MB wasm vs Slint's 8.7 MB.
  NativeAOT = own GC in linear memory → NO wasm-gc, none of the Kotlin
  adapter pain. Platform interfaces are [NotClientImplementable]/unstable →
  pin exact version like [[reference_slint_wasip2]].
- Effort ≈ 2–4 weeks (vs Slint's 2 days); derisking spike #1 = a bare C#
  guest exporting our `renderer` world via componentize-dotnet (~a day,
  kills the biggest unknown). Wait for a concrete need.

**ON DEVICE (task 107 part D, 2026-06-13)** — packaged as
`apps/user/wandr.avalonia.demo` (ships components/ui.wasm; on-device host
precompiles, NO dev-machine aarch64 AOT). Install = adb push wandrpkg →
`wandr-host --install` (precompile 41MB→88MB cwasm in **25s**, fine on
Pixel 2 XL) → `wandr-arbiter preload/launch`. Renders correctly on the
real panel under the live status bar (~174MB RSS). **HiDPI fix:** first
render was unreadably TINY (headless impl reports RenderScaling=1, so 1
logical-px=1 physical-px on a 1440 panel) → import `wandr:ui-shell/metrics`,
query `get-density()` once, apply a base `scale(density)` on the retained
canvas + size the window in LOGICAL units (physical/density) + convert
pointer coords by 1/density. Desktop density=2.0, device=panel density.
Touch verified via injected evdev taps (button counter incremented).
**DPI + miniature-copy bugs — RESOLVED.** Device showed (a) unreadably
small UI — headless impl reports RenderScaling=1 → fixed by density
base-scale on the retained canvas (import wandr:ui-shell/metrics,
get-density, scale per frame in BeginFrame, logical window size,
pointer/density); and (b) a miniature copy of the whole UI top-left that
appeared ON INTERACTION. ROOT CAUSE (found by logging InFrame per
CreateDrawingContext): input events (on-pointer/on-key) make Avalonia
render SYNCHRONOUSLY OUTSIDE the on-frame BeginFrame/EndFrame bracket where
the density Save+Scale isn't active → unscaled whole-UI render into the
retained canvas. FIX: gate the canvas — `CurrentCanvas => InFrame ?
_retained : null` — so out-of-frame renders no-op; all drawing is the
scaled on-frame pass (input still updates state; next frame's full repaint
via Window.InvalidateVisual + PreviousFrameIsRetained=false shows it). The
gate adds NO canvas save/restore. CRITICAL dead-ends: isolating the density
base per drawing-context (Save/Scale per ctx, Restore on Dispose) removed
the mini on DESKTOP but **SIGSEGV'd on the device aarch64 AOT** (compositor
push/pop unbalanced → canvas Restore underflow; depth-guarding didn't help;
a logical-offscreen+upscale variant also crashed, masked .NET exception in
CrashInfo.WriteChars). Full-redraw-ALONE was a mis-fix (helped at rest, mini
returned on interaction — the unscaled render re-issues per input event).
Verified Pixel 2 XL: clean after dragging through controls, stable 75s+.
Transient-overlay ghosts (tooltip/popup close) remain a separate latent
issue (popups out-of-scope).

**REUSABLE LIB EXTRACTED (2026-06-13).** `dotnet/avalonia-wandr/` = the
reusable C#/.NET guest UI adapter (new top-level `dotnet/`, the .NET peer
of `crates/`; documented in docs/repository-layout.md). App source now in
`apps/user/wandr.avalonia.demo/` (DemoApp.cs UI + AppInit.cs
[ModuleInitializer]→Host.Configure + 3-line csproj importing the props).
**Ships as SHARED SOURCE not a .dll** — componentize-dotnet generates WIT
bindings per-project against the world name, so the generated `GuestWorld`
namespace must be in the same assembly as the lib code; the .props compiles
src/ into the consumer against a FIXED world `wandr:avalonia-guest`. App
provides only Application + root Window. **IDLE CPU FIXED (on-demand
rendering, 2026-06-13): ~60% → ~3% on device.** Was: forced full repaint
(InvalidateVisual + PreviousFrameIsRetained=false) + full-surface
snapshot/blit/present every frame. Now: incremental retention
(PreviousFrameIsRetained=true, no InvalidateVisual) + on-demand present —
compositor early-outs when not dirty, CreateDrawingContext→FrameBridge.
MarkDrawn(), EndFrame skips acquire+snapshot+blit+present unless drawn; size
from on-resize (no idle buffer acquire). Mini stays fixed by the InFrame
gate (NOT the full redraw). Verified desktop 291960 frames/1 present, device
~3% idle, interaction fine. Headroom: frame-pacing WIT to stop max_fps
wake-ups ([[reference_on_demand_rendering]]) not wired. See
dotnet/avalonia-wandr/README.md.

**IME / soft keyboard — device-verified.** Wired like slint-wandr: import
the EXISTING `wandr:ui-shell/ime` (subset copy in wit/deps), call
`notify-editor-attached`/`-detached` on editor focus; typed text returns
via the already-wired key-handler. Avalonia resolves IME via
`TopLevel.PlatformImpl.TryGetFeature<ITextInputMethodImpl>()` but
HeadlessWindowImpl returns null + is internal (can't subclass) → instead
`WandrIme.Sync()` POLLS `FocusManager.GetFocusedElement() as TextBox` each
on-frame and reconciles (attach once on focus w/ input-type+Watermark+
char-offset selection; detach on blur; ESC→ClearFocus→detach; never
re-attach per keystroke). Verified Pixel 2 XL: tap TextBox → arbiter shows
[editor:text] + wandr.ime.keyboard [fg], typed text lands, tap-away hides
keyboard + counts the tap. Files: Platform/WandrIme.cs, ime import in
world.wit + wit/deps, Sync()+ESC in Exports.cs.
**Device input note:** `adb shell input` is gone under --no-art; inject
via evdev `sendevent` on the touchscreen node (Pixel2XL = /dev/input/event1,
1440×2880, type-B MT). Don't kill shared system apps (keyguard) for
screenshots — classifier blocks it. **Font fix (no-hardcode
rule):** WandrFontManager read-probes `/system-fonts` in order
(NotoSans-Regular→Roboto-Regular→DroidSans; bold falls back to regular —
device has no Roboto-Bold) instead of a desktop-only name. **Touch:**
`adb shell input` is GONE under --no-art (ART command); device input =
wandr-inputflinger evdev (physical tap = user confirms; desktop already
proved the pointer/key→Avalonia chain).

**SPIKE #2 + CONTROLS DEMO PASSED (task 107, 2026-06-13)** — Avalonia
11.3.17 + FluentTheme runs INTERACTIVE on the desktop host as a reactor
guest (`repros/avalonia-spike2/avalonia-demo`, 41.3 MB wasm, ~2-4 ms/frame;
button/checkbox/toggle/radio/slider/progressbar/textbox-typing/listbox all
verified via XTEST). Platform layer = `Platform/` (~1100 lines): render
iface + drawing ctx → wasi:canvas; Avalonia.Skia text stack de-Skia'd onto
HarfBuzzSharp-over-static-libharfbuzz; geometry = SVG strings; input via
HeadlessWindowExtensions. **Pins/gotchas:** (1) Avalonia 12.x net10 assets
need `TimeSpan.FromMilliseconds(long)` — missing from the componentize
ILC alpha corelib; newer ILC (preview.2.25509.1) traps at init → **pin
Avalonia 11.3.17 + ILC 10.0.0-alpha.1.25162.1**. (2) csproj needs
`<AvaloniaAccessUnstablePrivateApis>true`. (3) Implement **IRenderTarget2**
(IsSuitableForDirectRendering) or the compositor needs CreateLayer and
SWALLOWS the exception → zero draws. (4) Compositor renders INCREMENTALLY
assuming a retained target; wandr buffers arrive cleared → draw into a
persistent `graphics.new-offscreen` canvas, `snapshot()`-blit each present
(symptom otherwise: mouse-over leaves only the hovered control visible).
(5) .NET thread pool NEVER runs on single-threaded wasi (commit pipeline
survives via ExecuteSynchronously inlining). (6) hb 14.2.1 amalgamation +
`<DirectPInvoke Include="libHarfBuzzSharp">` serves BOTH HarfBuzzSharp and
raw P/Invoke. Popups/bitmaps/opacity-masks not implemented (loud throws).
Harfbuzz/glyphs text spike = `avalonia-spike2/text-guest` (task file).

**SPIKE #1 PASSED (task 106, 2026-06-12)** — `repros/avalonia-spike1/`:
bare C# reactor guest (import wasi:canvas types/draw/embedding, export
frame-handler) rendered the rect on the desktop host ON THE FIRST BUILD.
Release wasm **2.57 MB** (Debug 9.6), wasmtime AOT 0.64s→3.9MB cwasm,
host JIT load ~1s, render_frame 2.2–4.1ms. Zero canonical-ABI surprises
(resources→IDisposable handle classes, records/options/enums clean).
Friction: (1) export ns = `SpikeWorld.wit.exports.wasi.inputHandlers.v0_0_2`
— camelCased pkg + v-mangled version, build error names the expected
symbol; (2) released template pkg ships only componentize.wasi.cli —
hand-roll the lib csproj (OutputType=Library + `<Wit Include="wit"
World="spike"/>` + AssemblyName without dashes); (3) wasi-canvas subset
copy MUST include text.wit+scene.wit (worlds in canvas.wit reference
glyphs/layout/scene; resolver parses whole packages). Spike #2 =
harfbuzz-wasi link + WandrPlatformRenderInterface headless clone.
Full numbers: memo "Spike #1 results" + tasks/106-avalonia-spike.md.

**Toolchain check 2026-06-12 (for the spike session):** componentize-dotnet
latest = 0.7.0-preview00010; requires .NET 10+ SDK — available via apt on this
box (`sudo apt install -y dotnet-sdk-10.0`, USER must run, sudo needs password).
Linux NativeAOT-LLVM IS supported now (runtime.linux-x64...ilcompiler.llvm —
the memo's Windows-only risk is stale). No separate WASI-SDK: the NuGet
package downloads/caches wasm-tools+wac+wit-bindgen+ilcompiler on FIRST build
(slow, needs network). Setup: `dotnet new install
BytecodeAlliance.Componentize.DotNet.Templates` + `dotnet add package
BytecodeAlliance.Componentize.DotNet.Wasm.SDK --prerelease` (user-level, no
root). WIT wired via csproj `<Wit Update="..." World="..."/>`; export
namespace casing is the known fiddly bit. Spike #1 = bare C# guest exporting
wasi:input-handlers/frame-handler@0.0.2 + drawing one draw-rect via
wasi:canvas@0.0.2 (proposals/wasi-canvas/wit — post-Phase-C the 0.0.2 tree IS
wit/), run on the desktop host.
