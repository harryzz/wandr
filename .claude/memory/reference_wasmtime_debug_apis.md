---
name: wasmtime-debug-apis
description: "wasmtime 44 wasm-debugging APIs — call_hook, debug_exit_frames, breakpoints, single-step, debug_memory/global/function — and when to reach for each"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 52498412-67ad-4237-af3d-08469028c185
---

Wasmtime 44 has a fully-featured wasm-side debugger surface beyond what
we've been using. Worth knowing for the next time we need to bisect a
"some Kotlin code wrote to address X in linear memory" bug.

## Three tiers of precision

| Precision | API | Cost |
|---|---|---|
| Per host-call boundary | `Store::call_hook(closure)` — fires on CallingHost / ReturningFromHost / CallingWasm / ReturningFromWasm | negligible; already wired in wart-host under `profile` cargo feature |
| Per wasm function entry/exit | `Store::edit_breakpoints()` → `BreakpointEdit::add_breakpoint(module, pc)` at function-entry PCs | negligible at break cost; needs `Config::debug_info(true)` |
| Per wasm instruction | `BreakpointEdit::single_step(true)` | 100×–1000× slowdown when engaged; gate to suspect windows |

## Essential entry points (wasmtime 44 `runtime/debug.rs`)

```rust
// On Store<T>:
debug_exit_frames() -> impl Iterator<Item = FrameHandle>
debug_all_instances() -> Vec<Instance>
debug_all_modules() -> Vec<Module>
edit_breakpoints() -> Option<BreakpointEdit>
debug_register_module(&Module)
debug_register_component(&Component)

// On Instance:
debug_memory(store, MemoryIndex) -> Option<Memory>
debug_global(store, GlobalIndex) -> Option<Global>
debug_function(store, FuncIndex)
debug_table(store, TableIndex) -> Option<Table>
debug_tag(store, tag_index)
debug_shared_memory(store, MemoryIndex)

// On FrameHandle:
is_valid(store) -> bool
parent(store) -> Result<Option<FrameHandle>>
instance(store) -> Result<Instance>
module<T>(store) -> ...
wasm_function_index_and_pc(store) -> ...
num_locals(store) / num_stacks(store)
local(store, index) -> Result<Val>
stack(store, index) -> Result<Val>

// On BreakpointEdit:
add_breakpoint(&Module, ModulePC) -> Result<()>
remove_breakpoint(&Module, ModulePC) -> Result<()>
single_step(enabled: bool) -> Result<()>
breakpoints() -> Option<impl Iterator<Item = Breakpoint>>

// On Config:
debug_info(true)   // emits DWARF in compiled cwasm; gdb/lldb can attach
```

## When to reach for each

- **Find which host-call straddles a state change**: `call_hook` + read
  the suspect memory before and after each call. No code changes in the
  guest. We did this in task 30 for the State.magic1 transition.
- **Get a full wasm backtrace at a specific event**: from `call_hook`,
  call `store.debug_exit_frames()` and walk `FrameHandle::parent()` —
  emits `(module, function_index, pc)` per frame.
- **Catch the exact wasm instruction that writes a memory address**:
  single-step gated to the suspect window. Cost: tens of ms of wall
  time becomes seconds while engaged, so engage right before the bad
  call and disable as soon as the watched value changes. Then dump
  frames + `wasm_function_index_and_pc()`.
- **Attach native debugger (gdb/lldb)**: `Config::debug_info(true)`
  plus `Store::debug_register_module(&module)` causes wasmtime to emit
  DWARF for the compiled code and register it with the JIT-debug
  interface. Useful on desktop, not on Android NativeActivity where
  attaching a remote debugger is painful.

## Single-step gating recipe (the trick)

The single-step slowdown is huge, but our bugs are usually
"corruption happens once between adapter call N and N+1." So:

```rust
let mut step_armed = false;

store.call_hook(|cx, kind| {
    match kind {
        CallHook::ReturningFromWasm if state_was_clean_at_entry => {
            // Just exited an adapter call cleanly; arm single-step
            // for the gap before the next call.
            cx.as_store().edit_breakpoints().unwrap().single_step(true)?;
            step_armed = true;
        }
        CallHook::CallingWasm if step_armed => {
            // Entering another adapter call; disarm.
            cx.as_store().edit_breakpoints().unwrap().single_step(false)?;
            step_armed = false;
        }
        _ => {}
    }
    Ok(())
});
```

Plus a hook that fires on each step (need to set up via breakpoints
or via wasmtime's epoch_interruption — check current API). On each
step, check the watched memory; on first divergence, log frames and
disable.

## Cost / setup notes

- `call_hook` needs `wasmtime/call-hook` feature enabled. We have it
  on under the `profile` cargo feature in wart-host already.
- `debug_info(true)` and `edit_breakpoints()` need wasmtime built with
  debug instrumentation. May affect AOT cwasm contract — verify that
  our existing `.cwasm` was compiled with matching flags, or recompile.
- The wasmtime CLI itself can do `wasmtime run --gdb` for attach-based
  debugging, but we can't easily use that on-device.
- Components: `instance.debug_memory(MemoryIndex(0))` works on the
  inner module instance of a component. Need to enumerate via
  `debug_all_instances()` and pick the wart-app instance (not the
  adapter, which imports memory).

## What does NOT exist (as of v44)

- **Memory-write watchpoints** (set a watch on linear-memory address X,
  break on any wasm store that hits it). Have to emulate via single-step.
- **Source-line breakpoints from Kotlin/Wasm DWARF**. We have wasm-PC
  breakpoints; mapping Kotlin source line → wasm PC needs an external
  step.
- **Instruction-level instrumentation injection** (a la dynamorio /
  pin). `wasm-tools` doesn't have an `instrument` subcommand;
  third-party `wasm-instrument` exists but needs a rebuild.
