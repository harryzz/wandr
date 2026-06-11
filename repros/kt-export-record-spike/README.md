# kt-export-record-spike

**Question:** can a Kotlin/Wasm guest EXPORT a function that takes a record
containing strings (the `wasi:input-handlers` event shape), given that the
canonical ABI makes the HOST lower those strings into guest linear memory
via the guest's `cabi_realloc` *before* the export body runs? wandr's
standing rule said no — "host→guest exports primitive-only"
(`feedback_wasi_cabi_realloc_export_block`).

**Answer (2026-06-11): YES, with one ordering rule.** 100,000 randomized
calls (string sizes 0 B – 64 KB, multi-byte UTF-8, realloc-growth paths) on
the exact production stack — Kotlin 2.4.0-RC + the wandr `2.4.258-SNAPSHOT`
stdlib (KT-86415 Tier-2 fix) + the wandr-fork P1 adapter + wasmtime 45:

| export ordering | result |
|---|---|
| `freeAll()` → **lift args** → scoped allocations (official Kotlin/wit-bindgen order) | **100000/100000 OK** |
| `freeAll()` → scoped allocations → lift args (positive control) | **100000/100000 CORRUPTED** |

The positive control corrupting *every single call* proves three things at
once: the spike can see the hazard, the host-lowered args really do live in
the same arena the scoped allocator reuses after `freeAll`, and the safety
contract is purely **ordering**: an export wrapper must lift (read) every
argument into GC memory before the first `withScopedMemoryAllocator`
allocation. That is exactly the ordering JetBrains' official
`Kotlin/wit-bindgen` (branch `kotlin`) generator emits as of 2026-06.

So the old rule is superseded by a sharper one:

> Host→guest export args may carry strings/records/lists **iff** the
> binding lifts all args before any scoped (or realloc) allocation.
> `freeAll` at entry stays (it reclaims the host's arg allocations);
> lifting is pure reads, so freeAll-then-lift is safe.

## Layout

- `wit/spike.wit` — `wandr:spike/handler`: `key-event` record (2 strings +
  u32 + bool), two exports returning an FNV-1a-32 checksum: `on-key`
  (strict order) and `on-key-late-lift` (deliberately wrong order).
- `guest/` — Kotlin wasmWasi guest, hand-written export wrappers in the
  skiko style (`@WasmExport("wandr:spike/handler@0.1.0#on-key")`, flat
  params `(ptr,len, ptr,len, i32, i32) -> i32`).
- `runner/` — wasmtime-45 desktop JIT host: hammers both exports with a
  deterministic LCG stream and verifies checksums.

## Run

```bash
./build.sh           # 100k iterations
./build.sh 1000000   # more
```

## Caveats / not covered

- Desktop JIT only; arena behavior is wasm-semantics, not target codegen,
  so AOT/arm64 is expected to behave identically (re-verify if adopted in
  production bindings).
- Flat-params path only (≤16 flat params). A record big enough to spill to
  an indirect args area also arrives via `cabi_realloc` — same arena, same
  ordering rule expected, but not exercised here.
- Strings IN RESULTS (guest→host) were already known-good (guest lowers
  with its own allocator).
