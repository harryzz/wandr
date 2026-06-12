# Task 106 — AvaloniaUI spike #1: C# guest through the wandr contract

Memo: `docs/avalonia-wandr-feasibility.md` (+ memory `reference_avalonia_wandr`
— includes the 2026-06-12 toolchain check). Goal of spike #1: prove the
componentize-dotnet toolchain + WIT wiring + canonical ABI with a BARE C#
guest — no Avalonia yet.

## Prereq (user, one command)

```bash
sudo apt install -y dotnet-sdk-10.0     # Debian 13 feed has 10.0.301
```

## Toolchain (agent-installable, user-level)

- `dotnet new install BytecodeAlliance.Componentize.DotNet.Templates`
- project: `BytecodeAlliance.Componentize.DotNet.Wasm.SDK --prerelease`
  (0.7.0-preview00010). First build downloads + caches wasm-tools/wac/
  wit-bindgen/NativeAOT-LLVM ilcompiler (slow, needs network). Linux
  NativeAOT-LLVM is supported (`runtime.linux-x64...ilcompiler.llvm`).

## Steps

1. `dotnet new componentize.wasi.cli` hello → wasmtime runs it (toolchain ok).
2. Reactor guest with our WIT (csproj `<Wit Update="..." World="..."/>`):
   import `wasi:canvas/{types,draw,embedding}@0.0.2`
   (copies from `proposals/wasi-canvas/wit`), export
   `wasi:input-handlers/frame-handler@0.0.2` — on-frame does
   get-context → get-current-buffer → clear + one draw-rect → present.
   Known fiddly bit: wit-bindgen-dotnet export namespace casing.
3. Run on the desktop host (`WANDR_DESKTOP_SIZE=500x1000 …`), screenshot.
4. Record: binary size (memo predicts 30–80 MB class footprint for full
   Avalonia; the bare guest calibrates the floor), first-frame time,
   AOT-precompile time, any canonical-ABI surprises.

## Exit criteria

A rect on the desktop host from C#, sizes/timings noted, go/no-go update
to the feasibility memo (spike #2 = Avalonia `ISkiaGpu`-shaped renderer +
the HarfBuzz/glyphs text question).
