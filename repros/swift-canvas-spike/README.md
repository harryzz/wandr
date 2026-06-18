# swift-canvas-spike (task 114)

Proving a **Swift guest can be a wandr custom-WIT component** — the toolchain
unknown on the path to *CoreGraphics (OpenCoreGraphics) on `wasi:canvas`*. See
`tasks/114-swift-coregraphics-on-wasi-canvas.md` and
`docs/swift-openswiftui-wandr-feasibility.md`.

This is **P1**: a Swift analog of `repros/java-wasm-spike` — import one host
interface, export one function, over a custom WIT, run as a component in wasmtime.
P2 swaps the stand-in `host` interface for real `wasi:canvas` and implements
OpenCoreGraphics's (currently empty) `CGContext` over it.

## Status — ✅ P1 DONE (2026-06-18)

**Swift is a working wandr custom-WIT component guest.** Verified end to end with
Swift 6.3.2 + `swift-6.3.2-RELEASE_wasm` on wasmtime 45:

```
[host] calling Swift `run`…
[swift→host] log: hello from swift -> wasi (custom-WIT round trip)
[swift→host] draw-rect x=10 y=10 w=120 h=48 argb=0xFF3366CC
[swift→host] draw-rect x=24 y=70 w=96 h=96 argb=0xFFCC6633
[swift→host] draw-rect x=140 y=10 w=48 h=156 argb=0xFF33AA55
[host] run returned OK
```

Chain: Swift (`@_cdecl` export + C-interop imports over the `wit-bindgen c`
surface) → SwiftPM cross-build to a **wasip1 reactor** → `wasm-tools component new
--adapt …reactor.wasm` → a valid WASI 0.2 component → a wasmtime host
(`host/`) that provides WASI, implements `wandr:swift-spike/host`, and calls `run`.
This kills the "no public Swift custom-WIT precedent" unknown.

Reproduce: `./build.sh` (builds the Swift component **and** runs it via the host).

### Key build facts (the non-obvious bits)
- Use **SwiftPM** (`swift build --swift-sdk swift-6.3.2-RELEASE_wasm`), not raw
  `swiftc` — the SDK's `toolset.json` wires the sysroot + clang-rt; bare `swiftc`
  fails on `libclang_rt.builtins.a`.
- **Reactor** model: `-Xswiftc -Xclang-linker -Xswiftc -mexec-model=reactor` →
  `_initialize`, no `_start`, `run` stays exported (via the `wit-bindgen`
  `__export_name__`).
- **Link the component-type object:** `-Xlinker .../swift_spike_component_type.o`
  (the generated `.c` references it; it carries the embedded component type so
  `component new` needs no separate `embed`).
- Target triple is `wasm32-unknown-wasip1`.

## P2.1 DONE (2026-06-18) — Swift drives REAL wasi:canvas

The spike's `wit/` now imports the real `wasi:canvas/{types,draw,embedding}@0.0.2`
(subset copy in `wit/deps/`), and `Sources/SwiftSpike/spike.swift` exports
`render`: it takes the embedding handoff (`get-context` → `get-current-buffer` +
`present`), builds `paint` records, and calls `clear` / `draw-rect` / `draw-path`
— all via Swift C-interop over the `wit-bindgen c` surface. It compiles and
`wasm-tools component new --adapt` yields a **valid component importing
`wasi:canvas/{types,draw,embedding}`** and exporting `render`.

This answers the P2 de-risk: **Swift C-interop handles wasi:canvas's rich ABI**
(flat `paint` struct, `rect`, resource own/borrow handles) — not just P1's
scalars+string. Build: `./build.sh`.

### P1 vs P2 in this repo
- **P1** (custom-WIT round trip) is preserved: the self-contained runner is
  `host/` (a wasmtime host implementing a toy `wandr:swift-spike/host`, frozen
  against `host/wit/`). The full P1 result is at git `1d4fa6a7`.
- **P2** is the live direction (`wit/` = real wasi:canvas).

## P2.2 (2026-06-18) — Swift renders on wandr-host

`wit/` now exports the host-driven reactor surface
(`wasi:input-handlers/frame-handler` + `pointer-handler` + `wandr:ui-shell/
frame-pacing`) and imports `wasi:canvas`; `Sources/SwiftSpike/spike.swift`
implements `on_frame` (acquire the embedding buffer → `clear` + `draw-rect` +
`draw-path` → `present`). `package.toml` makes it the `wandr.swift.canvas.test`
app; `build.sh` emits `components/ui.wasm`.

Status: ✅ **DEVICE-VERIFIED 2026-06-18 (Pixel 2 XL).** Installed via
`wandr-host --install` and launched (`wandr-arbiter launch
wandr.swift.canvas.test`); logs show `eglSwapBuffers first call` → `rendered frame
0/1/2`, no traps. `adb exec-out screencap` confirms the pixels: dark background +
blue filled rect + green stroked triangle — exactly what `on_frame` draws. First
Swift guest rendering on wandr. (Desktop dev loop also runs it, but WSLg's weston
crashes in its bundled libpixman 0.43.2 — a WSLg bug, not the guest; use the
device.) Run it:

```bash
./build.sh                       # -> components/ui.wasm
# On WSLg, force winit onto X11/Xwayland — WSLg's weston intermittently segfaults
# in libpixman 0.43.2 on NATIVE-Wayland clients (microsoft/wslg#1386); X11 dodges
# it (Signal/Chrome/etc. are X11 clients and never hit it). Verified stable here.
WINIT_UNIX_BACKEND=x11 WANDR_DESKTOP_SIZE=480x800 \
  ../../runtime/wandr-host/target/x86_64-unknown-linux-gnu/release/wasm-android-host \
  components/ui.wasm
```

### Gotcha — don't cache the canvas-context `own` handle (from Swift)
Caching the `get-context` `own` handle in a global and re-borrowing it each frame
trapped on wandr-host with **`unknown handle index 0`** at `get_current_buffer`.
Acquiring the context **fresh each frame** (`get-context` → use → drop) works.
(The keyguard caches fine in Rust, so this is a wit-bindgen-c/Swift usage nuance —
revisit only if per-frame `get-context` ever shows cost.)

## P2.3 (2026-06-18) — Swift draws via CoreGraphics (CGContext over wasi:canvas)

`Sources/CoreGraphicsWasi/` implements OpenCoreGraphics's empty `CGContext` over
`wasi:canvas`: state stack (`saveGState`/`restoreGState` → `canvas.save`/`restore`),
CTM (`translateBy`/`scaleBy`/`rotate` → `translate`/`scale`/`rotate`), a current
path serialized to SVG path-data (`move`/`addLine`/`addQuadCurve`/`addCurve`/
`addRect`/`closePath`), fill/stroke color + line width resolved into a `paint`, and
`fill`/`stroke`/`fillPath`/`strokePath`/`clear`. The guest's `on_frame` now draws
with the **CoreGraphics API only** — no raw `wasi:canvas` in the guest.

✅ **DEVICE-VERIFIED (Pixel 2 XL)** — same scene as P2.2 (dark bg + blue rect +
green triangle), now via `CGContext`; `eglSwapBuffers` → rendered frames, no traps.

## P2.3b (2026-06-18) — vendored REAL OpenCoreGraphics, CGContext merged in

`Sources/OpenCoreGraphics/` is now the **actual upstream OpenCoreGraphics library
target** (MIT; `VENDORED.txt` records the commit), with its empty `CGContext.swift`
replaced by the wasi:canvas implementation and an added `CGColor` (upstream lacks
one). So the guest draws with OpenCoreGraphics's *own* types — `CGPath`/
`PathElement`/`CGLineCap`/`CGLineJoin` and Foundation's `CGPoint`/`CGRect` (OCG
gets the base CG types from Foundation: `@_exported import Foundation`).

✅ **DEVICE-VERIFIED (Pixel 2 XL)** — same scene, now via genuine OpenCoreGraphics.

**Build note:** OCG pulls in **Foundation/CoreFoundation**, which on wasm needs the
WASI emulation shims — `build.sh` adds `-D_WASI_EMULATED_{SIGNAL,MMAN,PROCESS_CLOCKS}`
+ `-lwasi-emulated-*`. **Cost:** the component jumps **~7 MB → ~60 MB** (Foundation
on wasm is heavy). For a production guest you'd avoid Foundation (a slim geometry
shim providing `CGFloat`/`CGPoint`/`CGRect`) — but that means patching OCG's
`public import Foundation`, so it's a separate decision. The self-contained P2.3
`CoreGraphicsWasi` variant (7 MB, no Foundation) is in git history if size matters.

## P2.3c (2026-06-18) — the 3 mapping "gaps" emulated (device-verified)

All three CGContext features that `wasi:canvas` lacks a direct verb for are
implemented in the vendored `CGContext` with **existing verbs only — no contract
change** (device-verified on Pixel 2 XL):

- **Line dash** (`setLineDash`) — guest-side path-walk: flatten to polylines, split
  into on/off runs per the pattern+phase, emit "on" runs as sub-paths → `draw-path`.
- **Offset+color drop shadow** (`setShadow`) — draw each shape twice: a `translate`d
  copy in the shadow color with `paint.blur` (mask-blur), then the real shape.
- **Alpha mask-clip** (`beginTransparencyLayer` + blend modes) — `save-layer`, draw
  the mask, then draw the content with **`src-in`** so it survives only where (and
  at the alpha) the mask covers. **Gotcha:** the dual order (content then mask with
  `dst-in`) only *dims* the mask region — because a drawn primitive's blend touches
  only its own pixels; mask-first + `src-in` (content covers the whole region) is
  the correct idiom.

So `wasi:canvas` is **sufficient for the full CoreGraphics 2D `CGContext`** — these
were the only mapping gaps.

Next: extend the `CGContext` surface (gradients, images, text). SwiftUI
(OpenRenderBox + OpenSwiftUI) remains the out-of-scope wall.

## Prerequisite (the gate)

```bash
# Swift toolchain + the wasm32-unknown-wasi Swift SDK (swift.org / swiftwasm).
# e.g. via swiftly, then:
swift sdk install <swift-wasm-sdk-url>
swift sdk list           # confirm a wasm32-unknown-wasi SDK is present
```

> Suggested: run the install yourself from the prompt with `!` so the output lands
> here, e.g. `! swift sdk install <url>`.

## Build (once the SDK is installed)

```bash
./build.sh
# swiftc -target wasm32-unknown-wasi ... -> spike.core.wasm
# wasm-tools component new --adapt ...reactor.wasm -> spike.component.wasm
```

`build.sh` is a starting point — expect iteration on the reactor link flags
(`-mexec-model=reactor`) and on whether the component-type section comes from the
linked `swift_spike_component_type.o` (canonical wit-bindgen-c flow) vs a separate
`wasm-tools component embed`.

## Then (P1 host)

A small wasmtime **component** host that implements `wandr:swift-spike/host`
(`log`, `draw-rect`) and calls the exported `run` — the analog of
`java-wasm-spike/stub-host`'s `comp-host`. Proves the round trip end to end.

## Files

- `wit/spike.wit` — the minimal custom WIT (import `host`, export `run`).
- `generated/` — `wit-bindgen c` output (the C ABI surface) + `module.modulemap`.
- `Sources/spike.swift` — the guest: `@_cdecl` export + C-interop imports.
- `build.sh` — Swift → wasip1 → component pipeline.
