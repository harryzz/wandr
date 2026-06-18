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

### Next — P2.2 (render) and P2.3 (CGContext)
- **P2.2 render:** add the `wasi:input-handlers/frame-handler` export (+
  `frame-pacing`) and a `package.toml`, then run on **wandr-host** (which renders
  `wasi:canvas` with skia) on the desktop dev loop (task 101) → real pixels. No
  bespoke host needed — wandr-host already implements the contract.
- **P2.3 CGContext:** implement OpenCoreGraphics's empty `CGContext` over exactly
  these `wasi:canvas` calls (the mapping in
  `docs/swift-openswiftui-wandr-feasibility.md`).

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
