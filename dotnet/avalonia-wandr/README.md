# avalonia-wandr

Run [AvaloniaUI](https://avaloniaui.net/) (C#/.NET, NativeAOT→wasm) as a
wandr guest. The .NET analogue of `crates/slint-wandr` — a shared
guest-side UI-framework adapter that renders through `wasi:canvas`, shapes
text with harfbuzz, and wires input + the soft keyboard through the
`wasi:input-handlers` / `wandr:ui-shell` contracts.

## What it provides

- `Platform/` render backend: `IPlatformRenderInterface` /
  `IDrawingContextImpl` → `wasi:canvas`; Avalonia's text stack de-Skia'd
  onto HarfBuzzSharp over a statically-linked harfbuzz; geometry as SVG
  path strings.
- The `wandr:avalonia-guest` WIT world (`wit/`) + its exports
  (`frame`/`pointer`/`key` handlers) and the runtime that drives Avalonia
  from the host's callbacks (`src/Host.cs`).
- DPI scaling from `wandr:ui-shell/metrics`, soft-keyboard wiring via
  `wandr:ui-shell/ime`, and the harfbuzz wasm archive (`native/`).

## Distribution model: shared source, not a .dll

componentize-dotnet generates the WIT bindings **per project** against the
world name, so the generated `GuestWorld` namespace must live in the same
assembly as the library code that uses it. A precompiled `.dll` therefore
can't share those types across the assembly boundary. So avalonia-wandr
ships as **shared source** compiled into the consuming component via
`avalonia-wandr.props`, against a **fixed** world name (`wandr:avalonia-guest`).

## Consuming it

A consumer's `.csproj`:

```xml
<Project Sdk="Microsoft.NET.Sdk">
  <Import Project="<rel>/dotnet/avalonia-wandr/avalonia-wandr.props" />
  <PropertyGroup>
    <AssemblyName>my_app</AssemblyName>
    <RootNamespace>my_app</RootNamespace>
  </PropertyGroup>
</Project>
```

Provide an `Application` + root `Window`, and register them before the
first frame:

```csharp
internal static class AppInit
{
    [System.Runtime.CompilerServices.ModuleInitializer]
    internal static void Init() =>
        WandrAvalonia.Host.Configure(() => new MyApp(), MyApp.BuildMainWindow);
}
```

Build → `bin/Release/net10.0/wasi-wasm/native/<assembly>.wasm`; install as
a wandrpkg (`components/ui.wasm`). Reference consumer:
`apps/user/wandr.avalonia.demo`.

Pins (see comments in `avalonia-wandr.props`): Avalonia **11.3.17**
(12.x needs a newer corelib than the componentize ILC alpha ships) and
`AvaloniaAccessUnstablePrivateApis` (the platform interfaces are internal).

## Known issues

- **High idle CPU (~60%).** The render loop repaints **every frame**
  (`Window.InvalidateVisual()` + `ForceRenderTimerTick`, no dirty/on-demand
  gating) — it renders continuously even when nothing changed. Full redraw
  is currently load-bearing (it's what suppresses the unscaled
  input-render artifact, task 107), so the fix is on-demand rendering:
  render only when Avalonia signals dirty / via frame-pacing, the way the
  Rust guests do (`reference_on_demand_rendering`). Not yet investigated.
- Transient overlays (tooltips/popups/menus) are separate top-levels and
  out of scope — they can leave brief residue and aren't wired to arbiter
  surfaces.
