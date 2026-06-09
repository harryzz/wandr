# Architecture: the host/guest boundary

This doc clarifies how the wandr-host (Rust) and a wandr guest (Kotlin/Wasm,
or any other WASM Component-Model guest) call into each other, what
"host-driven" means in practice, and why the design is shaped that way.

It exists because the same question keeps coming up: "what is
`renderFrame(nanos)` — is it inlined? is it wasip2? where does the loop
live?" The short answers are below; the long answers follow.

## TL;DR

- **Two crossing directions, both real function calls:**
  - **Host → guest** (host calls a function the guest exports). Example:
    `renderFrame(nanos)`.
  - **Guest → host** (guest calls a function the host imports). Example:
    `draw_rect(...)`.
- **Nothing is inlined.** Both directions cross the WASM Component-Model
  canonical-ABI boundary. There's lowering on the way in, lifting on the
  way out.
- **"Host-driven" means:** the host owns the render loop, vsync timing,
  input draining, lifecycle dispatch, and the scheduler. The guest is
  reactive — it provides function bodies the host invokes on its
  schedule. The loop lives in `wandr-host/src/standalone.rs`, not in any
  guest.
- **wasip2 ≠ "anything WASM Component-Model".** wasip2 is a specific
  set of standardized WIT interfaces (`wasi:io`, `wasi:filesystem`,
  `wasi:cli`, `wasi:sockets`, etc.). `my:skiko-gfx/*` is custom WIT —
  same plumbing, different namespace, not part of wasip2.

## What `renderFrame(nanos)` actually is

`renderFrame` is a **guest export** — a function the Kotlin guest
declares (with `@WasmExport`), which becomes part of the component's
exported interface. The host invokes it via wasmtime's
`Func::call`/`call_render_frame`:

```
[ host loop owns timing — wandr-host/src/standalone.rs ]
    ↓ (host invokes via wasmtime; canonical-ABI lowering of `nanos`)
[ guest's renderFrame body runs — Compose recomposition + draw calls ]
    ↓ (guest, while running, calls host imports like draw_rect, draw_text)
[ host implements those imports in wandr-host/src/canvas_impl.rs —
  skia-safe draws to GPU ]
    ↓ (control returns up through the import calls;
       eventually renderFrame returns; canonical-ABI lifting; void result)
[ host loop sleeps until next frame, then calls renderFrame again ]
```

So a single frame involves many boundary crossings:

1. Host → guest: one `renderFrame(nanos)` call.
2. Guest → host: dozens of `draw_*` / `create_text_blob` / etc. calls
   while renderFrame's body executes.
3. Guest returns; host gets control back; sleeps; repeats.

## The two crossing directions

| Direction | Guest-side declaration | Host-side wiring | Examples |
|---|---|---|---|
| **Host → guest** (host calls guest) | guest declares `@WasmExport` (or a WIT `export` interface that wit-bindgen turns into one) | host calls via the generated wasmtime binding (`SkikoUi::call_render_frame`, `Command::wasi_cli_run().call_run`, …) | `renderFrame(nanos)`, `onPointerEvent(...)`, `onLifecycleChanged(state)`, `wasi:cli/run.run()` |
| **Guest → host** (guest calls host) | guest declares `@WasmImport` (or a WIT `import` interface that wit-bindgen turns into one) | host registers in the wasmtime `Linker` via `add_to_linker` | `draw_rect`, `draw_text`, `create_text_blob`, `markdown.render`, `wasi:filesystem/types.open_at` |

Both use the WASM Component Model's canonical ABI. The same machinery
applies regardless of direction — argument lowering/lifting, resource
handles, string copies, list allocations, the lot.

## What wasip2 is and isn't

wasip2 (WASI Preview 2) is a **standardized set of WIT interfaces** that
ship as part of the WASI spec. It covers things like:

- `wasi:io/{poll, streams}` — async I/O primitives
- `wasi:filesystem/{types, preopens}` — filesystem access
- `wasi:cli/{environment, exit, stdin, stdout, stderr, run, command}` —
  command-line program shape
- `wasi:sockets/{tcp, udp}` — network sockets
- `wasi:clocks/{wall-clock, monotonic-clock}` — time
- `wasi:random/random` — RNG

Our skiko-gfx interface (`my:skiko-gfx/canvas`, `.../window`,
`.../lifecycle`, etc.) is **custom WIT** — same WIT language, same
Component-Model machinery, but a separate namespace, designed and
implemented by us. It is NOT part of wasip2.

Our cross-app dep interface (`wandr:markdown/renderer`,
`wandr:emoji/shaper`, …) is also custom WIT, defined by the app/system
component authors.

This matters because:

- When wandr-host calls `wasmtime_wasi::p2::add_to_linker_sync(&mut linker)`,
  it's registering the wasip2 set in the Linker — but NOT skiko-gfx, and
  NOT any custom app interfaces. Those have to be added separately.
- A consumer can import any mix: some wasip2 (e.g. for stdio), some
  skiko (for drawing), some custom (for cross-app deps). All are wired
  via the same `Linker` API; the namespace is just metadata.

## The host-driven design

The wandr-host design rule: **timing, orchestration, and resource
lifetimes live in the host. The guest provides function bodies; nothing
more.**

Concretely, the host owns:

- The render loop (`standalone.rs:run_cwasm_loop`'s `loop { ... }`).
- Vsync timing (the `frame_target = 16ms` sleep).
- Input draining (`sf.poll_input` → `dispatch_pointer_v2` / `dispatch_android_key`).
- Lifecycle dispatch (`on_lifecycle_changed` calls based on screen
  on/off + SIGTERM/SIGINT/SIGHUP).
- The scheduler (`scheduler.drain_due` → `on_scheduled_callback`).
- EGL/Skia setup, SurfaceFlinger surface acquisition, binder init.
- The wasmtime `Engine` + `Store` lifetime, GC scheduling, resource
  tables.

The guest does:

- Whatever happens inside `renderFrame(nanos)`: Compose recomposition,
  emitting draw calls via the imported skiko canvas.
- Whatever happens inside `onPointerEvent`/`onKeyEvent`: state mutation,
  invalidating layers.
- Whatever happens inside `onLifecycleChanged`: Compose lifecycle observer
  callbacks.

The guest never:

- Owns a `while (true)` loop driving its own frames.
- Schedules its own vsync.
- Polls for input events on its own clock.
- Decides when to pause/resume.

## Why we did NOT adopt the "universal world with guest-driven `run()`" model

A tempting alternative is the WASI command shape: every guest exports
ONE function (`wasi:cli/run.run`), the host calls it once, the guest's
body contains the entire program (including any loops).

Under that model, a Compose guest would look like:

```kotlin
fun main() {
    setupRenderer()
    while (!shouldExit()) {
        val nanos = waitForNextVsync()    // host import — blocks
        renderFrame(nanos)
        for (ev in pollInputEvents()) {   // host import — drains queue
            handle(ev)
        }
    }
}
```

This puts the frame loop in the **guest**. Pros: one world fits every
app shape; cleaner manifest contract. Cons:

- The loop, timing, input cadence, and lifecycle responsiveness move
  into Kotlin. The host loses direct control of timing decisions.
- Vsync would need to be exposed as a pollable WASI primitive — more
  surface, more failure modes.
- The Compose runtime would need restructuring to drive itself instead
  of being driven.
- It moves work into the guest that the host can do equally well.

The standing decision is: **keep the host in charge.** Compose apps
keep the multi-export reactive shape (renderFrame + onPointerEvent +
onLifecycleChanged + ...). The host calls them on its own schedule.

## One-shot consumers (CLI/smoke) fit the host-driven model with cardinality 1

A wasi:cli/run consumer like `wandr-app-md-smoke` doesn't have a render
loop. Its program is:

```kotlin
fun main() {
    val result = render("# Hello")   // calls cross-app dep via @WasmImport
    if (result.blocksLen <= 0) throw IllegalStateException(...)
    // implicit return → exit 0
}
```

The host invokes `wasi:cli/run.run()` **once** — same primitive as one
`renderFrame()` invocation, just with cardinality 1 instead of 60×/sec.
The guest still doesn't drive a loop; it just runs to completion. The
"host drives" rule holds.

The `wandr-host --run-once <app-id>` mode is the entry for this shape.
It uses the same `WandrLoader::load` + same `wire_markdown_dep` proxy
setup as the Compose path — only the final instantiate-and-invoke call
differs (`Command::instantiate` + `call_run` instead of
`SkikoUi::instantiate` + the render loop).

## Cross-app deps wire identically regardless of consumer cardinality

`wire_markdown_dep` in `app_loader.rs` doesn't know or care whether the
consumer is Compose or CLI. It:

1. Builds a dep-local Linker (WASI only — markdown imports no skiko).
2. Calls `RendererWorld::instantiate(store, dep_component, dep_linker)`
   — the dep lands in the consumer's Store.
3. Clones the dep's `Guest` accessor (a wasmtime Func handle).
4. Registers the dep's `render` export as a proxy entry in the
   consumer's Linker via `linker.instance("wandr:markdown/renderer@0.1.0").func_wrap("render", ...)`.

When the consumer later instantiates (either as SkikoUi or Command), its
import of `wandr:markdown/renderer@0.1.0` is satisfied by that proxy. Each
call from the consumer goes: consumer → proxy closure → dep instance's
`call_render` → dep body → return back through proxy → consumer.

The plumbing is symmetric across consumer shapes. That's the whole point
of doing it at Linker level rather than wiring into the consumer-specific
binding code.

## See also

- `wandr-host/src/app_loader.rs` — `load_dep_components` + `wire_dep_into_linker`
  + `wire_markdown_dep`.
- `wandr-host/src/standalone.rs` — the canonical host-driven loop for
  Compose consumers.
- `wandr-host/src/run_once.rs` — the host-driven one-shot for CLI
  consumers (task 36 step 7).
- `tasks/36-cross-app-deps.md` — scope + status of the cross-app dep work.
- `post-art-roadmap.md` §7 + §9 — packaging + isolation model.
