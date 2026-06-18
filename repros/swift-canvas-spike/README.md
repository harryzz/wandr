# swift-canvas-spike (task 114)

Proving a **Swift guest can be a wandr custom-WIT component** — the toolchain
unknown on the path to *CoreGraphics (OpenCoreGraphics) on `wasi:canvas`*. See
`tasks/114-swift-coregraphics-on-wasi-canvas.md` and
`docs/swift-openswiftui-wandr-feasibility.md`.

This is **P1**: a Swift analog of `repros/java-wasm-spike` — import one host
interface, export one function, over a custom WIT, run as a component in wasmtime.
P2 swaps the stand-in `host` interface for real `wasi:canvas` and implements
OpenCoreGraphics's (currently empty) `CGContext` over it.

## Status

- **P0 done (no Swift needed):** the WIT (`wit/spike.wit`) and the **C binding
  surface** Swift imports are generated and verified — `wit-bindgen c` produced
  `generated/swift_spike.{h,c}` + `swift_spike_component_type.o`. The surface is
  clean C: imports Swift calls (`wandr_swift_spike_host_log`,
  `wandr_swift_spike_host_draw_rect`) and one export it implements
  (`exports_swift_spike_run`). The Swift side (`Sources/spike.swift`, via `@_cdecl`
  + C-interop) and `build.sh` are written and ready.
- **P1 blocked on the Swift WASM SDK** (not in this environment). Install, then run
  `./build.sh`.

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
