# `-C collector=copying` traps with `wasm trap: cast failure` after sustained allocation

> Draft bug report for **bytecodealliance/wasmtime**. **Not yet posted** — review
> before submitting. Companion to the cyclic-leak finding in this repro's README.

## Summary

Running a real WasmGC module (Java compiled via TeaVM) under the copying
collector (`-C collector=copying`) traps with `wasm trap: cast failure` once the
workload has allocated enough to trigger a collection. The **same module runs
correctly under the default `drc` collector**. Simple, low-allocation entry points
run fine under `copying` too — the failure only appears after sustained allocation
(i.e. after the collector has relocated live objects), which points at a
reference/typing bug in object relocation rather than anything specific to the
guest.

This matters because `copying` is currently the only collector that reclaims
cycles (`drc` cannot), so it's the path out of the well-known DRC cyclic-garbage
leak — and it's unusable here.

## Environment

- wasmtime `45.0.0 (377cd917a 2026-05-21)`, CLI.
- GC code is unchanged through `45.0.1`/`45.0.2` (those releases only fix WASIp2
  clocks and a WASIp1 `fd_renumber` leak), so this applies to all of 45.0.x.
- Host: x86_64-linux. Also reproduced cross-compiled to aarch64-linux-android
  (same trap class).
- Guest: a WasmGC module from TeaVM (`WEBASSEMBLY_GC_WASI`-style output) —
  standard typed `struct`s, no linear-memory GC. Imports only
  `wasi_snapshot_preview1.{fd_write, clock_time_get}`.

## Steps to reproduce

The guest is a tight loop that allocates one small object per iteration and prints
a counter every 1,000,000 iterations (`repros/java-leak-repro` in this tree;
exact source there). Any TeaVM WasmGC module with a hot allocation loop + numeric
`String` formatting should do.

```bash
# default collector — runs fine (modulo the known DRC cyclic-garbage growth)
wasmtime run -W all-proposals=y --invoke run java-leak-repro.wasm

# copying collector — traps shortly after the first GC
wasmtime run -W all-proposals=y -C collector=copying --invoke run java-leak-repro.wasm
```

## Expected

The copying collector is a complete tracing collector; relocating live objects
should preserve their types and update all references. The module should keep
running (as it does under `drc`).

## Actual

It prints the first line, then traps during the first counter format (a
`StringBuilder` numeric append) — i.e. on the first heap touch *after* a
collection has moved objects:

```
leak-repro(java): self-driving repro starting -- pure allocation loop, WASI stdout only
Error: failed to run main module `java-leak-repro.wasm`

Caused by:
    0: failed to invoke `run`
    1: error while executing at wasm backtrace:
    0:   0x126a - <unknown>!java.lang.AbstractStringBuilder::insert_2
    1:   0x202a - <unknown>!java.lang.StringBuilder::insert_3
    2:   0x1fc4 - <unknown>!java.lang.StringBuilder::insert_1
    3:   0x1fd5 - <unknown>!java.lang.StringBuilder::insert@caller_1
    4:   0x1a67 - <unknown>!java.lang.AbstractStringBuilder::append_1
    5:    0xf70 - <unknown>!java.lang.StringBuilder::append_1
    6:   0x2448 - <unknown>!testapp.Main::run
    2: wasm trap: cast failure
```

The trap is a `ref.cast` inside `StringBuilder` — a long-lived object whose type
appears wrong after relocation. `StringBuilder` is just where it surfaces; the
trigger is "do a typed cast on an object after the copying GC has run."

## What works (narrowing it down)

Under `-C collector=copying`, low-allocation entry points on the same toolchain
run correctly:

| entry point | behavior under `copying` |
|---|---|
| constant-string `println` (no GC pressure) | ✅ prints |
| return a `long` (`clock_time_get`) | ✅ correct |
| pure `i32` compute (`packed-len`) | ✅ correct |
| hot allocation loop + numeric `String` format | ❌ `cast failure` after ~1M allocs |

So it's not basic GC support — it's relocation: the failure needs enough
allocation to force at least one collection, then a `ref.cast` on a surviving
object. (A non-cyclic variant of the same loop did not trap within a 5s window but
also made no visible progress — possibly heavy slowdown or a stall; reported as a
secondary observation, not the primary bug.)

## Why it matters

- `drc` cannot collect reference cycles (by design), so cyclic garbage leaks
  unboundedly — see this repro's README (RSS 0 → multi-GB in seconds).
- `copying` is the cycle-collecting alternative, but the above makes it unusable
  for a real toolchain's output.
- Net: there is currently no production collector that reclaims cycles for this
  class of guest.

## Offer

I can provide a smaller standalone `.wasm` (and `.wat`) that reproduces the
`ref.cast`-after-collection failure without the surrounding repro, if that helps
bisect the copying collector's relocation path.
