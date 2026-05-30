---
name: wasi-adapter-state-corruption
description: "Tooltip-on-wasi SIGILL (task 29/30) is wasi-preview1 adapter State.magic1 corruption at linear-memory 0x10008, magic1 overwritten with a Kotlin/Wasm class-header word. RESOLVED 2026-05-21 by task 34 Option B (fixed linear-memory partition) — see [[kotlin-wasm-scopedmemory-destroy-bug]]."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 52498412-67ad-4237-af3d-08469028c185
---

**RESOLVED 2026-05-21 (task 34).** The "candidate proper fix" #3 below —
move State to a region Kotlin's allocator never touches — was implemented
as Option B: a static linear-memory partition. The adapter's `State::new`
now pins `State` at the fixed address `[0x10000,0x20000)` and the Kotlin
stdlib's root `ScopedMemoryAllocator` starts at `RESERVED_BASE=0x20000`,
so the region is never reused. Device-verified — no corruption, no SIGILL,
and the `State::with` self-heal workaround has been **removed**. Full
detail in [[kotlin-wasm-scopedmemory-destroy-bug]]. Everything below is the
historical diagnosis.

---

The Material3 TooltipBox SIGILL diagnosed end-to-end in task 29 is NOT
a `poll_oneoff` precondition violation from kotlinx-coroutines `Delay`.
The actual failing assert in the WASI preview1 reactor adapter is at
`crates/wasi-preview1-component-adapter/src/lib.rs:2805`:

```rust
fn with(f: impl FnOnce(&State) -> Result<(), Errno>) -> Errno {
    let state_ref = State::ptr();
    assert_eq!(state_ref.magic1, MAGIC);   // ← THIS fails
    assert_eq!(state_ref.magic2, MAGIC);
    ...
}
```

`State::with` is the gate for every adapter operation, so the failure
manifests on whichever wasi call follows the corruption — in the
Tooltip case, `poll_oneoff` from `withTimeout`. The 6 candidate
`poll_oneoff` asserts from task 29's static analysis are red herrings.

Diagnostic captured on device (task 30 Step 3, 2026-05-19):

| field | value | interpretation |
|---|---|---|
| `magic1` | `0x3E2BA0FC` | corrupt (was `0x21686775` = "ugh!") |
| `magic2` | `0x21686775` | intact |
| `ptr` (State address) | `0x10008` | linear-memory offset of State |
| MAGIC constant | `0x21686775` | `u32::from_le_bytes(*b"ugh!")` |
| `count` (State::with calls before corrupt) | 3 | reproducible across runs |
| corruption detected at | entry of call#3 | exit of call#2 was clean |

The third State::with call (long-press → Tooltip → Delay → poll_oneoff)
sees corruption on entry. Calls #1 and #2 exited cleanly. So corruption
fires during the Kotlin/Wasm activity BETWEEN call#2 exit and call#3
entry — i.e., outside the wasi adapter, during Compose rendering of the
~30 s session before the long-press. The exact same corrupting value
(`0x3E2BA0FC`) reproduces across runs — a stable Kotlin class-header
word that gets written at offset 0x10008.

**Why:** The adapter's static `State` struct (`PAGE_SIZE` = 64 KB)
is allocated once at cold init via `cabi_realloc` → Kotlin/Wasm's
`reallocAllocator`. The pointer is stashed via `set_state_ptr`. Kotlin's
allocator does NOT treat that allocation as a long-lived root — at some
point on the Tooltip path the State region is reused for a fresh Kotlin
object, and that object's class-header word overwrites `magic1`. Only
the first 4 bytes are clobbered (magic2, 64 KB later, is intact), which
matches a tiny-object header write at offset 0.

**Same family as** [[wasi-realloc-allocator-pollution]] and
[[currentnanotime-pollutes]] — both document that Kotlin/Wasm's
realloc-allocator state is fragile across WIT-import boundaries.

**How to apply:** When triaging "SIGILL inside adapter / function[32]"
or "wasmtime trap not intercepted" on wasi:
1. Don't bisect the calling poll_oneoff path — chase the State magic.
2. Read 4 bytes at linear-memory `0x10008`. If they're not
   `0x75 0x67 0x68 0x21` ("ugh!"), State is corrupt and the next
   adapter call WILL trap regardless of input shape.

**Working workaround (verified on device 2026-05-19):** patch the
adapter's `State::with` to self-heal — when `state_ref.magic1 != MAGIC
|| state_ref.magic2 != MAGIC`, call `State::init(get_state_ptr())`
in place before the assert. This re-initializes the State at the same
linear-memory address (preserving the get_state_ptr-stored pointer).
File descriptors held by State get reset, which is fine for wart-app
(no persistent fd I/O through the adapter). End-to-end test: Tooltip
test #28 long-press completes cleanly, DatePicker chevrons `<` `>`
work; small lag after long-press (State::init cost), then normal.

**Soak (2026-05-19, 30 manual long-presses + scrolling + DatePicker
chevron interactions, ~3 min):**
- 0 SIGILL
- 0 recovery messages after the initial cold-boot recovery (the
  workaround re-inits State exactly once early and then State stays
  healthy under sustained Tooltip use; Kotlin's allocator does not
  re-corrupt the same offset)
- haptics fires on Material3 long-press → confirms the Compose
  gesture path completes
- no render_frame ok=false, no leaks observable in this window

Workaround is a fork of upstream wasi-preview1-component-adapter.
Lives at `wasmtime-src/crates/wasi-preview1-component-adapter/src/lib.rs`
(v44.0.1 tag + diagnostic + self-heal in State::with). Must be
re-embedded into every wart-app cwasm via
`wasm-tools component new --adapt`.

**Candidate "proper" fixes (not yet pursued):**
- Custom kotlinx-coroutines `Dispatcher.Delay` that bypasses the wasi
  adapter — only fixes Tooltip-class paths, not other wasi calls.
- Pin adapter's State allocation in Kotlin/Wasm allocator (upstream
  Kotlin runtime change).
- Move State to a memory region Kotlin's allocator never touches
  (upstream adapter change).

**Diagnostic plumbing** that captured this (kept in tree):
- `wart-host/src/wasi_stderr.rs`: synchronous `LogcatStderr StdoutStream`
  routes guest stderr to `log::warn!(target: "wasi_stderr")` from inside
  the wasi `fd_write` host call — needed because wasmtime-wasi's default
  inherit_stderr buffers async and SIGILL aborts before the worker flushes.
- `wasmtime-src/crates/wasi-preview1-component-adapter/src/lib.rs`:
  added `with_count: Cell<u32>` to State (adjust `temporary_data_size`
  by `2*size_of::<usize>()`), `DIAG corrupt#N`/`DIAG exit-corrupt#N`
  blocks bracketing the assert in `State::with`, and made `eprint_u32`
  `pub(crate)`. Built from v44.0.1 tag with dev profile +
  `RUSTFLAGS=-C debuginfo=2` (release profile force-strips at workspace
  Cargo.toml:675). Do NOT use `static mut` here — `wasm-tools component
  new --adapt` rejects adapters with a `start` section, which Rust emits
  for static mut zero-init via `__wasm_init_memory`.
- New adapter: `wasmtime-src/target/wasm32-unknown-unknown/debug/wasi_snapshot_preview1.wasm`.
  Re-embed via `wasm-tools component new --adapt …`; re-AOT for aarch64.
