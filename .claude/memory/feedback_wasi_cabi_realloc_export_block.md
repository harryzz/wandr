---
name: wasi-cabi-realloc-export-block
description: "SUPERSEDED 2026-06-11 by repros/kt-export-record-spike: host→guest exports CAN carry records-with-strings on the Tier-2 stdlib (2.4.258) IFF the wrapper lifts ALL args before any scoped allocation (freeAll→lift→scoped, the official Kotlin/wit-bindgen order) — 100k/100k pass; late-lift corrupts 100k/100k. The old 'primitive-only' rule was the right call on the OLD stdlib (cabi_realloc threw at lowering time); shipped IME contract stays primitive until someone needs richer."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: c7a4384f-c3b0-4cdf-98cb-aa514fa75079
---

When a host-side wasmtime typed call into a guest's EXPORTED WIT
function carries a record-with-strings param, the lowering invokes
the guest's `cabi_realloc` to allocate string buffers in linear
memory. In Kotlin/Wasm, `componentModelRealloc` (called by
`cabi_realloc`) throws:

```
thrown Wasm exception
  exception message: Can't create new allocators while
                     realloc-allocated memory is not freed
```

Reproducer (task 49 step 1b, wandr.ime.keyboard):

- WIT: `interface ime { on-editor-attached: func(info: editor-info); }`
  where `editor-info` is a record containing two strings
  (`hint`, `initial-text`) plus an enum + two u32s.
- Host calls `ime_events.war_ime_ime().call_on_editor_attached(
  &mut store, &wit_info)` — wasmtime lowers the record by calling
  `cabi_realloc(0, 0, 1, hint.len())` then `cabi_realloc(0, 0, 1,
  text.len())` on the guest BEFORE invoking the user's
  `@WasmExport` function.
- The first `cabi_realloc` throws the exception.

**Why:** between render-frame calls, the IME guest's realloc
allocator state is left polluted by something. Could be the
scheduler-callback path, `Random.Default` initialization, an
@WasmImport call from skiko, or some other allocation that doesn't
properly clean up. The `freeAllComponentModelReallocAllocatedMemory()`
that every @WasmImport callsite invokes (per
[[wasi-realloc-allocator-pollution]]) only handles the IMPORT
direction; the EXPORT direction's `cabi_realloc` runs from
wasmtime's lowering code path that the guest doesn't control.

**What DIDN'T work:**

- Adding `freeAllComponentModelReallocAllocatedMemory()` at the END
  of render-frame (after `withScopedMemoryAllocator { … }` returns).
  Diagnostic logged confirmed the freeAll runs every 600 frames,
  but the next host→guest call still threw. Something between
  frames re-pollutes faster than render-frame's end-of-frame cleanup.
- A minimal-body `@WasmExport` (just `ImeEventsImpl.recordInputTypeTag(p0)`,
  no allocator scope, no string lift) STILL threw — confirming the
  issue is in the canonical-ABI lowering BEFORE the user function
  body runs.

**Workaround (shipped in task 49 step 1b):** simplify the WIT
contract so host→guest calls take only PRIMITIVE params (enums,
u32, etc.) — no records, no strings. The IME's `on-editor-attached`
became `on-editor-attached: func(input-type: input-type)` — a
single enum param. wasmtime lowers it as 1 i32, never calls
`cabi_realloc`, never throws.

Hint, initial-text, selection-bounds were dropped from the host→guest
direction. If/when a future IME genuinely needs them (e.g. for
autocorrect context, prediction), they can flow back the guest→host
direction via the `input-connection` interface (which has
`get-text-before-cursor` / `get-text-after-cursor` verbs) where the
`freeAll-at-start` pattern works.

**How to apply:** when designing a new host→guest WIT EXPORT
call, prefer primitives. If a record is required (e.g. for an
atomic batch update), keep its fields primitive — no strings, no
nested records that flatten to strings. Strings on this direction
will trigger this bug until the root pollution source is found.

**Why:** end-of-frame `freeAll` proved insufficient; the
pollution source is some other call path between frames. Until
the root cause is fixed, host→guest record-with-strings is a
landmine. Primitive params sidestep the entire `cabi_realloc`
codepath.

**RESOLVED 2026-06-11 — `repros/kt-export-record-spike`:** on the
production stack (2.4.0-RC compiler + **2.4.258-SNAPSHOT Tier-2 stdlib** +
wandr-fork adapter + wasmtime 45), the host lowering `cabi_realloc` calls
NO LONGER throw (the "can't create new allocators" guard was the OLD
allocator design; Tier-2's persistent realloc allocator removed it), and
host→guest record-with-strings args survive **100,000/100,000** randomized
calls (0 B–64 KB, multi-byte UTF-8) — **iff the export wrapper lifts every
arg before the first `withScopedMemoryAllocator` allocation**
(freeAll → lift → scoped; the order JetBrains' Kotlin/wit-bindgen emits).
The positive control (freeAll → scoped scribble → lift) corrupted
100,000/100,000 — args really do sit in the arena scoped allocs reuse, so
the ordering IS the entire safety contract. New rule: rich export args are
allowed; lift-before-alloc is mandatory; primitives remain fine where they
ship today (IME contract unchanged). Caveats: desktop-JIT spike,
flat-params path only (≤16); re-verify on AOT/arm64 + the indirect-args
spill case before adopting in production bindings.

**Historical notes below (old stdlib era) — kept for the failure modes:**

**To revisit later** (open question):

- Find what's polluting the realloc allocator between render-frame
  calls. Candidates: on-scheduled-callback, Random.Default in
  Compose's initial composition, async dispatch via
  WasiFrameDispatcher, some WIT @WasmImport that doesn't free
  properly.
- Potential fix sites: (a) make `cabi_realloc` itself robust to
  pollution (auto-freeAll then allocate); (b) add freeAll-on-exit
  to ALL @WasmExport functions, not just render-frame;
  (c) Kotlin/Wasm stdlib patch that decouples scoped-allocator
  state from realloc-allocator state.
- Until resolved, the workaround scales — task 49 step 1b shipped
  with primitive params only, and step 2/3/4 don't need anything
  richer.

Related:
- [[wasi-realloc-allocator-pollution]] — the IMPORT-direction
  version of this pattern, well-understood + worked-around with
  the `freeAll-at-start of every @WasmImport` rule.
- [[canonical-abi-import-export-asymmetry]] — the canonical-ABI
  asymmetry between IMPORT (caller-allocated return area) and
  EXPORT (callee-allocated). This memory is the EXPORT direction's
  failure mode specifically.
