# Task 34 — KT-86415: fix the adapter-State use-after-free via a fixed linear-memory partition (Option B)

**Status:** ✅ DONE — device-verified 2026-05-21.
**Scoped:** 2026-05-20. Spun out of task 30 and the KT-86415 investigation.

> **Outcome (2026-05-21):** Option B shipped as specified. Kotlin stdlib
> `2.4.258-SNAPSHOT` (root `ScopedMemoryAllocator` starts at
> `RESERVED_BASE=0x20000`, `destroy()` stock); adapter fork `State::new`
> pins `State` at `STATE_BASE=0x10000`. Pre-flight confirmed `wart-app.wasm`
> has zero static linear data (`(memory 0)`, no `(data)` segments) so
> `[0,0x20000)` is free — constants used unchanged. Win 1: DatePicker
> chevrons + Tooltip long-press → 0 SIGILL / 0 corruption, verified on a
> build with the `State::with` self-heal **removed**. Win 2: idle
> wasm-linear-memory leak 0.111 MB/s vs the known-good 2.4.257 baseline's
> 0.114 MB/s — identical, no regression; residual is the pre-existing
> wasmtime-DRC leak (#13403), out of scope. Self-heal removed; init.d
> override kept (points at 2.4.258) until KT-86415 lands upstream; the
> superseded 2.4.255/256/257 stdlib snapshots were deleted from mavenLocal.

---

## TL;DR

The WASI preview1 component adapter's 64 KB `State` is allocated via the
Kotlin module's exported `cabi_realloc` (= Kotlin's
`componentModelRealloc`). It therefore lives in the **same linear-memory
bump region** that Kotlin's `ScopedMemoryAllocator` owns and reuses — a
classic use-after-free (KT-86415).

**Fix (Option B): statically partition linear memory.**
- `[0, R)` — reserved; the adapter's `State` lives here.
- `[R, ∞)` — Kotlin's `ScopedMemoryAllocator` / `componentModelRealloc`.

Two coordinated one-constant changes:
- **Kotlin stdlib** (`MemoryAllocation.kt`): the root `ScopedMemoryAllocator`
  starts at `R`, not `0`.
- **Adapter** (`State::new`): place `State` at a fixed address inside
  `[0, R)` instead of calling `cabi_realloc`.

This kills the use-after-free **and** the leak, with **no heuristic** —
it is a trivially-correct static partition. It is a genuine, minimal,
upstreamable change (the rejected watermark patch was a heuristic; this
is not).

---

## Background — the bug (KT-86415)

The full history is in [KT-86415](https://youtrack.jetbrains.com/issue/KT-86415/),
`tasks/30-wasi-adapter-assert-and-wasmtime-signal-handler.md`, and the
memory `feedback_kotlin_wasm_scopedmemory_destroy_bug.md`. Short version:

- Kotlin/Wasm's `wit-bindgen` fork calls
  `freeAllComponentModelReallocAllocatedMemory()` aggressively between
  WIT calls, because it assumes `realloc` memory is only ever short-lived
  copy-buffer scratch (as the Canonical ABI describes).
- The WASI preview1 adapter instead uses the exported `cabi_realloc` as a
  generic allocator for its **arbitrarily-long-lived `State`**.
- After a `freeAll`, the next `withScopedMemoryAllocator` hands back the
  same address range and overwrites `State` — surfacing as adapter-State
  corruption (the magic-canary asserts), a SIGILL on Android, Tooltip /
  DatePicker-chevron crashes.

JetBrains' position (KT-86415, *Investigating*): `ScopedMemoryAllocator`
is correct; this is a use-after-free; the spec is ambiguous about whether
`realloc` may be used as a long-lived allocator.

---

## What was already tried — do NOT retry

### 2.4.255 — `destroy()` parent-bump  (shipped stopgap; leaks)
`ScopedMemoryAllocator.destroy()` propagates the child scope's
high-water mark to the parent, so no address is ever reused. Fixes the
UAF, but **never reclaims any scoped memory** → a memory leak. JetBrains
rejected it as a fix ("makes everything a memory leak"). Currently
shipped + the adapter-fork `State::with` self-heal, as a stopgap.

### Tier 2 — persistent `reallocAllocator` + watermark `freeAll`  (FAILED on device)
Idea: keep `reallocAllocator` alive forever; first `freeAll` records a
"permanent" watermark; later `freeAll`s rewind to it. Verified PASS on a
simplified standalone repro — then **crashed on device at
`render_frame #0`**: `ScopedMemoryAllocator is suspended when nested
allocators are used`.

An on-device allocator trace (instrumented stdlib, see "Environment" §)
proved **two independent fatal flaws**:
1. `componentModelRealloc` is **always** called inside an open
   `withScopedMemoryAllocator` scope — every realloc event in the trace
   had scope depth ≥ 1 (`R<1,…>` / `R<2,…>`, never `R<0,…>`). A
   persistent `reallocAllocator` is *suspended* the instant a scope
   nests on it → `allocate()` throws.
2. The first `freeAll` **precedes** the first `realloc`, so a watermark
   captured at the first `freeAll` is `0` and protects nothing.

**Do not reattempt any persistent-allocator / watermark / "tell
long-lived from per-call realloc" scheme.** The trace proved the stdlib
*cannot* distinguish the adapter's long-lived `State` realloc from
per-call marshalling realloc — both are `componentModelRealloc(0,0,size)`,
interleaved inside scopes from frame 0. There is no signal.

---

## The architecture constraint (why Option B is the shape of the fix)

- The adapter does **not have its own linear memory**. It imports
  `__main_module__`'s (the Kotlin module's) memory — it must, because
  every preview1 call hands it pointers into the Kotlin module's memory.
  **Adapter and Kotlin share one linear memory.**
- Kotlin's root allocator is `ScopedMemoryAllocator(0, parent = null)` —
  it owns that memory **from address 0 upward** and bump-climbs without
  bound.
- Therefore there is **no region of the shared memory** the adapter can
  place `State` in that Kotlin's allocator will not eventually reuse —
  *unless* the two sides agree a static partition. That agreement is
  Option B.

Note: Kotlin/Wasm with WasmGC keeps managed objects (incl. `String`) in
the GC heap, not linear memory — linear memory is used almost entirely
by `withScopedMemoryAllocator` / `componentModelRealloc` plus whatever
static data segments the module linker emits. That is why the allocator
can start near 0 at all.

---

## Option B — design

### Linear-memory layout (recommended constants)

```
[0,            STATE_BASE)  module static data + null guard   (page 0)
[STATE_BASE,   STATE_BASE+0x10000)  the adapter's State        (page 1, exactly 64 KB)
[RESERVED,     ∞)           Kotlin ScopedMemoryAllocator / componentModelRealloc
```

Recommended: `STATE_BASE = 0x10000` (65536), `RESERVED = 0x20000`
(131072). Page-aligned, `State` gets exactly its own page, address 0
stays null, page 0 holds the module's static data segments.

**Hard contract:** the Kotlin-side `RESERVED` and the adapter-side
`STATE_BASE` must satisfy `STATE_BASE + 0x10000 ≤ RESERVED` and both
must be agreed/documented. They live in two repos — drift = corruption.

**Verify before committing the constants:** confirm the Kotlin module's
static linear data fits below `STATE_BASE`. Inspect the built
`wart-app.wasm` data segments / `__data_end` with `wasm-tools print` (or
`wasm-tools objdump`). wart-app's static linear data is expected to be
small (WasmGC keeps objects off linear memory), but if it exceeds
`STATE_BASE`, raise `STATE_BASE` (and `RESERVED`) to the next page above
`__data_end`.

### Kotlin stdlib change (`~/xl/kotlin/.../wasm/unsafe/MemoryAllocation.kt`)

The root allocator must start at `RESERVED` instead of `0`. In
`createAllocatorInTheNewScope`:

```kotlin
// before:
val allocator = currentAllocator?.createChild() ?: ScopedMemoryAllocator(0, parent = null)
// after:
private const val RESERVED_BASE = 0x20000  // KT-86415: see task 34 — reserved for the WASI adapter's State
...
val allocator = currentAllocator?.createChild() ?: ScopedMemoryAllocator(RESERVED_BASE, parent = null)
```

That is the **entire** stdlib change. `componentModelRealloc` reaches
`createAllocatorInTheNewScope` too, so per-call realloc also starts
above `RESERVED`. **Keep `destroy()` STOCK** — Option B does not need
(and must not include) the 2.4.255 parent-bump; the allocator stays
LIFO, so per-call memory is reclaimed and there is **no leak**.

### Adapter change (`~/wart/wasmtime-src/crates/wasi-preview1-component-adapter/src/lib.rs`)

`State::new()` (≈ lib.rs:2845) currently calls the imported
`cabi_realloc`. Change it to place `State` at the fixed `STATE_BASE`:

```rust
#[cold]
fn new() -> *mut State {
    assert!(matches!(unsafe { get_allocation_state() }, AllocationState::StackAllocated));
    unsafe { set_allocation_state(AllocationState::StateAllocating) };

    // KT-86415 (task 34): State lives at a fixed reserved address, NOT
    // via cabi_realloc. cabi_realloc == Kotlin's componentModelRealloc,
    // which places it in the reclaimable bump region → use-after-free.
    // The Kotlin ScopedMemoryAllocator is built to start at RESERVED
    // (0x20000); [STATE_BASE, STATE_BASE+0x10000) is excluded from it.
    const STATE_BASE: usize = 0x10000;
    let ret = STATE_BASE as *mut State;

    // Ensure linear memory covers the reserved region before writing.
    let need_pages = (STATE_BASE + size_of::<State>()).div_ceil(PAGE_SIZE);
    let have = core::arch::wasm32::memory_size(0);
    if have < need_pages {
        core::arch::wasm32::memory_grow(0, need_pages - have);
    }

    unsafe { set_allocation_state(AllocationState::StateAllocated) };
    unsafe { Self::init(ret); }
    ret
}
```

(Exact `memory_grow` form: confirm the adapter already uses
`core::arch::wasm32` elsewhere; the adapter is `no_std`-ish — match its
existing idiom.)

The `State::with` self-heal added in task 30 becomes dead code under
Option B (State can no longer be corrupted). **Keep it during bring-up**
as a tripwire — if `wart fork: wasi adapter State corruption — recovered`
ever appears in logcat under an Option-B build, the partition is wrong.
Remove it only after B is device-verified.

Out of scope: the adapter's *stack* allocation (`AllocationState`
`Stack*`) is transient and has never been the corruption victim — leave
it. If stack corruption ever appears, that is a separate follow-up.

---

## Build & deploy

Stdlib (Option B variant — stock + the `RESERVED_BASE` constant ONLY,
**no** `destroy()` patch). Publish under a fresh version so it does not
clobber 2.4.255/256/257:

```bash
cd ~/xl/kotlin
# edit MemoryAllocation.kt as above; set defaultSnapshotVersion=2.4.258-SNAPSHOT in gradle.properties
./gradlew :kotlin-stdlib:publishWasmWasiModulePublicationToMavenLocal --console=plain --no-daemon
```

Point the override at it:
`~/.gradle/init.d/kt-86415-stdlib-override.gradle.kts` → `useVersion("2.4.258-SNAPSHOT")`.

Adapter fork:
```bash
cd ~/wart/wasmtime-src
cargo build -p wasi-preview1-component-adapter --target wasm32-unknown-unknown --release
```

wart-app + pipeline (per `CLAUDE.md`): `compileProductionExecutableKotlinWasmWasi`
→ `wasm-tools component embed` → `component new --adapt <fork adapter>`
→ `wasmtime compile` (aarch64) → `adb push` the cwasm → restart.

The skiko / 31 compose-*-wasi klibs do **not** need rebuilding for a
stdlib swap — `withScopedMemoryAllocator` is `@DoNotInlineOnFirstStage`,
so its body is re-lowered at the final wart-app link; the
`createAllocatorInTheNewScope` change is a non-inline callee resolved at
that link. (Confirmed during the Tier 2 attempt.) Relink wart-app only.

---

## Verify

Win condition — Option B is correct iff BOTH hold on device, on a build
whose stdlib has **no `destroy()` patch**:

1. **No use-after-free.** Interact with TooltipBox / DatePicker chevrons
   (the task-30 SIGILL triggers). No SIGILL, no crash, and **no**
   `wart fork: wasi adapter State corruption — recovered` line in
   logcat. (If that line appears, the partition constants are wrong.)
2. **No leak.** Sustained interaction does not grow wasm linear memory
   unboundedly. With a stock LIFO allocator (no parent-bump), per-call
   marshalling memory is reclaimed every frame. Sanity-check with the
   task-23 profiling hooks or by sampling `memory.size` over a few
   minutes of use.

If both hold: Option B is the real fix — remove the init-script
override entirely once the change lands upstream/clean, drop the
`State::with` self-heal, and retire the 2.4.255/256/257 snapshots.

---

## Environment state at hand-off (2026-05-20)

- **Device:** running the `2.4.257-SNAPSHOT` cwasm = 2.4.255 `destroy()`
  parent-bump logic + dormant allocator-trace instrumentation. Working
  (`render_frame ok=true`). This is the safe fallback build.
- **mavenLocal `kotlin-stdlib-wasm-wasi`:** `2.4.255-SNAPSHOT` (destroy
  parent-bump), `2.4.256-SNAPSHOT` (Tier 2 — abandoned, do not use),
  `2.4.257-SNAPSHOT` (2.4.255 logic + trace instrumentation).
- **`~/.gradle/init.d/kt-86415-stdlib-override.gradle.kts`:** redirects
  `kotlin-stdlib-wasm-wasi` → `2.4.257-SNAPSHOT` for all builds.
- **`~/xl/kotlin`:** working tree clean (stock 2.4.0-RC). This is where
  the Option B stdlib edit goes.
- **Adapter fork:** `~/wart/wasmtime-src/crates/wasi-preview1-component-adapter/`
  — has the task-30 `State::with` self-heal; otherwise upstream. Built
  release artifact at
  `target/wasm32-unknown-unknown/release/wasi_snapshot_preview1.wasm`.
- **Standalone repro:** `~/wart/kt-memalloc-repro/` (Codeberg) — models
  the realloc/freeAll/scope sequence. NOTE: it under-modeled reality
  (missed realloc-during-open-scope) and gave Tier 2 a false PASS. If
  used to pre-validate Option B, first extend it to allocate at a fixed
  low address and confirm a `withScopedMemoryAllocator` starting at
  `RESERVED` never touches it.

---

## First actions for a fresh session

1. Read `feedback_kotlin_wasm_scopedmemory_destroy_bug.md` and task 30.
2. `wasm-tools print` the current `wart-app.wasm` — confirm static
   linear data fits below `STATE_BASE = 0x10000`; adjust constants if not.
3. Make the two changes (Kotlin `RESERVED_BASE`, adapter `State::new`).
4. Build stdlib `2.4.258-SNAPSHOT`, adapter fork, wart-app; deploy.
5. Verify both win conditions on device.
