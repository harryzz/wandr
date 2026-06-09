# Task 39 — generic dep wiring via wasmtime introspection

> **Status:** 🟡 in progress — picks up alongside task 40 (emoji picker)
> because we now have N=2 system components, so the "first concrete
> dep wired by name; refactor to registry once N>1" note from task 36
> is due.

## Why

`wandr-host/src/app_loader.rs` currently dispatches dep wiring via a
hardcoded match:

```rust
fn wire_dep_into_linker(...) {
    match dep.interface.as_str() {
        "wandr:markdown/renderer@0.1.0" => wire_markdown_dep(linker, store, dep),
        other => bail!("...not yet wired in wandr-host..."),
    }
}
```

Every new system component → edit wandr-host, regenerate bindgen module,
rebuild + deploy host. That defeats the cross-app dep package boundary
(which task 36 built).

## What changes

Replace `wire_dep_into_linker`'s match dispatch + the per-dep
`wire_*_dep` functions + the per-dep `bindgen!{}` modules in `lib.rs`
with **one generic function** that uses wasmtime's component
introspection APIs:

1. Build a dep-local Linker (WASI only — same as today).
2. Instantiate the dep into the consumer's Store.
3. Walk the dep's `Component::component_type().exports(engine)` for
   exported interfaces.
4. For each exported function in each interface, register a closure
   on the consumer's Linker via `LinkerInstance::func_new(name, ty,
   handler)`. The closure forwards calls via dynamic `Val`-based
   `Func::call(store, params, results)` + `Func::post_return(store)`.

The dep's component carries its full WIT type info — no `.wit` file
needs to ship in the wandrpkg. The host introspects the binary at load
time.

## Trade-offs

- **Pro:** truly extensible. New system components install + run with
  zero wandr-host changes.
- **Pro:** decouples package distribution from host distribution.
- **Pro:** one code path instead of N hardcoded match arms.
- **Con:** dynamic `Val`-based dispatch instead of typed bindgen. For
  a one-shot `render()` it's nothing; for 60Hz calls it would add a
  small but real per-call overhead. None of our current deps are 60Hz.
- **Con:** loses compile-time type safety on the host side. The host
  trusts the dep's WIT contract as encoded in the component binary.

## Files affected

- `wandr-host/src/app_loader.rs` — `wire_dep_into_linker` becomes
  generic; `wire_markdown_dep` deleted.
- `wandr-host/src/lib.rs` — `markdown_bindings` module deleted.
- No new files. No installer changes. No manifest schema changes.

## Verification

1. Local build: `cargo build` clean — confirms the introspection API
   shape compiles.
2. Markdown smoke (existing test): `MarkdownCard` still renders the 11
   blocks correctly after the refactor — proves backwards-compat.
3. Emoji smoke (task 40 driver): new `EmojiCard` renders a grid via
   the generic path — proves it works for a NEW dep with zero added
   wiring code in wandr-host.

## Out of scope

- Resource type handling (`Linker::resource(...)`) — neither markdown
  nor emoji has WIT resources. Add when a dep needs one.
- Async dep dispatch — wandr-host is sync today; deferred when needed.
- Lazy / on-demand dep instantiation — separate-Store + OnceCell, see
  `tasks/36-cross-app-deps.md` "Where true lazy linking *would* have
  helped".
