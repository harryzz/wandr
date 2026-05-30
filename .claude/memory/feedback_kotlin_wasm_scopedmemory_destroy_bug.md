---
name: kotlin-wasm-scopedmemory-destroy-bug
description: "wasi-adapter State corruption on Kotlin/Wasm (KT-86415). A use-after-free: the WASI preview1 adapter held canonical-ABI `realloc` memory across Kotlin's `freeAllComponentModelReallocAllocatedMemory`. RESOLVED 2026-05-21 by task 34 Option B — a fixed linear-memory partition (stdlib root allocator starts at 0x20000, adapter State pinned to [0x10000,0x20000)). 2.4.255 destroy()-patch and Tier 2 watermark were both rejected stopgaps."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 52498412-67ad-4237-af3d-08469028c185
---

## RESOLVED — OPTION B SHIPPED + DEVICE-VERIFIED (2026-05-21, task 34)

The KT-86415 use-after-free is **fixed**. Option B = a static
linear-memory partition agreed by both sides of the shared linear
memory:

- **Kotlin stdlib** (`MemoryAllocation.kt`, `createAllocatorInTheNewScope`):
  the root `ScopedMemoryAllocator` starts at `RESERVED_BASE = 0x20000`
  instead of `0`. `destroy()` is left **stock** — LIFO reclaim is
  preserved, so there is **no per-call leak** (unlike the rejected
  2.4.255 parent-bump). Published as `kotlin-stdlib-wasm-wasi:2.4.258-SNAPSHOT`.
- **Adapter fork** (`State::new`): places the 64 KB `State` at the fixed
  address `STATE_BASE = 0x10000` (no `cabi_realloc`), `memory.grow`-ing
  to cover it. `State` is exactly one page, so `[0x10000,0x20000)` is
  its window — exactly the region Kotlin's allocator now skips.

The two constants satisfy `STATE_BASE + size_of::<State>() == RESERVED_BASE`.
No heuristic; trivially correct.

**Device verification (Pixel 2 XL, 2026-05-21):**
- Win 1 — no UAF: DatePicker `< >` chevrons + TooltipBox long-press (the
  task-30 SIGILL triggers) exercised → 0 SIGILL, 0 crash. Confirmed on a
  build with the task-30 `State::with` self-heal **removed** — so the
  partition itself is correct, not masked by recovery.
- Win 2 — no leak: idle wasm-linear-memory growth 0.111 MB/s, identical
  to the known-good 2.4.257 baseline (0.114 MB/s) → Option B introduces
  no regression. The residual ~0.11 MB/s idle climb is the pre-existing,
  out-of-scope wasmtime-DRC continuation leak ([[wasmtime-drc-no-autoschedule]]),
  exercised by the focused TextField's cursor-blink frame loop.

**Build wiring:** the `2.4.258` stdlib is redirected into all builds via
`~/.gradle/init.d/kt-86415-stdlib-override.gradle.kts`. That override
**stays in place** until KT-86415 lands in a released Kotlin — removing
it makes builds resolve stock Kotlin and the UAF returns. The
superseded `2.4.255/256/257-SNAPSHOT` mavenLocal artifacts were deleted;
only `2.4.258-SNAPSHOT` remains. Adapter fork now differs from upstream
only by the `State::new` change (self-heal removed). See `tasks/34-kt86415-fixed-partition.md`.

Everything below is the historical record of the investigation; its
root-cause framing was corrected upstream and the chosen fix is the
Option B above.

---

## UPSTREAM CORRECTION — KT-86415 (JetBrains, 2026-05-20)

Jim Teichgräber (JetBrains, Wasm backend) responded on
[KT-86415](https://youtrack.jetbrains.com/issue/KT-86415/) — State:
*Investigating*. He **rejects this memory's original framing.** Read
this before acting on anything below:

- **`ScopedMemoryAllocator` is correct.** Scoped addresses are *meant*
  to be invalid outside their scope; our reproducer (comparing
  addresses across scopes) "behaves exactly as expected, that's not a
  bug." The `destroy()` "doesn't propagate the range" framing is wrong.
- **Real cause = a classic use-after-free.** Kotlin's wit-bindgen fork
  calls `freeAllComponentModelReallocAllocatedMemory` frequently
  because it assumes the exported `realloc` is used *only* as the
  Canonical ABI describes — short-lived copy buffers. The WASI
  preview1 component adapter instead uses `realloc` as a **generic
  allocator for arbitrarily-long-lived memory** (its `State`). The
  first `freeAll` then makes every later read of that memory UB.
- **Spec-ambiguous whose fault it is.** `CanonicalABI.md` neither says
  its listed `realloc` uses are exhaustive nor forbids others.
  JetBrains "need[s] to decide whether to allow arbitrary reallocs."
- **Our `destroy()` parent-bump patch is rejected as a fix.** Jim:
  never-freeing just makes the UAF read immutable memory so it "just
  works" until the first repeated allocation — "not a solution to
  use-after-free, just defining UB conveniently (and making everything
  a memory leak)." Also fragile with multiple parallel child
  allocators at the same scope.

**Status of our patch (`kotlin-stdlib-wasm-wasi:2.4.255-SNAPSHOT`):**
empirically works — device-verified, corruption gone — but it is a
*leak-trade stopgap*, not a correct fix. The proper fix is adapter-side
(don't hold canonical-ABI `realloc` memory across a `freeAll`) or an
upstream Kotlin decision to support arbitrary `realloc`. Until then:
patch + the adapter-fork `State::with` self-heal is the pragmatic
combination.

The detailed investigation below is kept as the record of *what we
observed and shipped* — but its root-cause attribution is superseded
by the correction above.

---

## TIER 2 ATTEMPTED + FAILED — OPTION B IS THE PATH (2026-05-20, task 34)

Two fixes were explored after the upstream correction:

- **Tier 2** (persistent `reallocAllocator` + watermark `freeAll`):
  verified PASS on the standalone repro, then **crashed on device** at
  `render_frame #0` (`ScopedMemoryAllocator is suspended`). An on-device
  allocator trace proved two fatal flaws: (1) `componentModelRealloc` is
  *always* called inside an open `withScopedMemoryAllocator` scope, so a
  persistent realloc allocator is suspended when scopes nest on it;
  (2) the first `freeAll` precedes the first `realloc`, so a watermark
  captures nothing. **Do not reattempt any persistent-allocator /
  watermark / "distinguish long-lived from per-call realloc" scheme** —
  the stdlib has no signal to tell them apart; they are byte-identical
  and interleaved from frame 0.

- **Architecture finding:** the WASI adapter and the Kotlin module
  **share one linear memory** (the adapter imports `__main_module__`'s
  memory; it has none of its own). Kotlin's `ScopedMemoryAllocator` owns
  it from address 0 upward. So no in-memory relocation of the adapter's
  64 KB `State` is safe without a partition agreed by *both* sides.

- **Chosen path — Option B (fixed linear-memory partition):** reserve
  `[0, R)` for the adapter's `State`; Kotlin's allocator starts at `R`
  instead of `0`. One constant each side (stdlib `RESERVED_BASE` +
  adapter `State::new`), no heuristic — kills the UAF *and* the leak.
  Full design + build/verify steps in
  `~/wart/tasks/34-kt86415-fixed-partition.md`.

Repro caveat: `kt-memalloc-repro` under-modeled reality (it never
exercised `componentModelRealloc` interleaved with an open scope) and so
gave Tier 2 a false PASS. Trust device verification over the repro.

---

In Kotlin/Wasm stdlib's `kotlin.wasm.unsafe.MemoryAllocation.kt`:

```kotlin
internal fun destroy() {
    destroyed = true
    parent?.suspended = false   // ← does NOT bump parent.availableAddress
}

internal fun createChild(): ScopedMemoryAllocator {
    val child = ScopedMemoryAllocator(availableAddress, parent = this)
    suspended = true
    return child
}
```

When a child scope's `destroy()` runs, the parent's `availableAddress` is **not** advanced to reflect what the child allocated. The bytes the child wrote sit in memory but are reusable by the next child opened from the same parent.

This is **correct for ephemeral scoped allocations** (the design intent of `withScopedMemoryAllocator`). It's **broken for long-lived allocations** that the WASI preview1 adapter makes via `cabi_realloc`/`componentModelRealloc`, because:

1. `componentModelRealloc` uses `createAllocatorInTheNewScope()` (which builds on `ScopedMemoryAllocator`).
2. The adapter expects the returned memory to persist forever (it stashes the pointer in a wasm global and reads from it for the program's lifetime).
3. Any subsequent `withScopedMemoryAllocator` from another part of the program — after `freeAllComponentModelReallocAllocatedMemory()` resets the chain — gets a child of the same parent, with the same `availableAddress`, and **overwrites the adapter's State block**.

**Empirical verification (task 30 watchpoint, 2026-05-19):**
- Probe A allocated 8 bytes via `withScopedMemoryAllocator` → ptr = 8.
- Audio test's `writePcmF32` binding then allocated 76 KB in its own scope, then destroyed.
- Probe B (same code as A, fresh scope) → ptr = **8** again.

If parent had advanced, B would have been ~`76808`. It wasn't. Hypothesis confirmed.

**Same scope hierarchy seen via the linear-memory data**: the WASI adapter's State at `0x10008` is overwritten by the audio buffer that starts at `~0xFFB8` even though State.with #1, #2 had passed cleanly — because the parent of State's allocator and the parent of audio's allocator are the same scope, with the same forgotten `availableAddress`.

**Filed upstream:** [KT-86415](https://youtrack.jetbrains.com/issue/KT-86415/) — track here for status. Standalone reproducer published at <https://codeberg.org/harryzz/kt-memalloc-repro>.

**Patch validated locally 2026-05-20**: built patched Kotlin stdlib from `~/xl/kotlin` (v2.4.0-RC tag with the `destroy()` parent-bump diff), published to mavenLocal as `kotlin-stdlib-wasm-wasi:2.4.255-SNAPSHOT`, ran the second-version repro against stock + patched stdlibs side by side. Stock: long-lived block overlapped by `newScope.allocate`. Patched: high-water mark propagated; no overlap. **Residual edge case** noted in `~/wart/kt-memalloc-repro/kt-86415-fix-completeness.md` (also published in the Codeberg repro repo): if `componentModelRealloc` is ever called with no active outer `withScopedMemoryAllocator` scope, the patch is a no-op (parent is null). Doesn't manifest in current Kotlin/Wasm code paths but worth flagging on the YouTrack issue.

**Related upstream issues:**
- `KT-65030` "K/Wasm: memory allocator for Component Model ABI" — Fixed in 2.4.0-Beta1, added `componentModelRealloc` etc. But the underlying `ScopedMemoryAllocator` semantics weren't changed, so the bug persists for adapter-style long-lived allocations.
- The fix needs `destroy()` to bump parent's `availableAddress` by `(this.availableAddress - this.startAddress)`, OR `reallocAllocator` to be backed by a separate non-scoped persistent pool.

**Workaround in our project:** the WASI preview1 adapter fork at `wasmtime-src/crates/wasi-preview1-component-adapter/` has a self-heal in `State::with` that re-`init`s State when magic1/magic2 are corrupted. See [[wasi-adapter-state-corruption]].

**Deployed-build state (2026-05-20):** the patched stdlib (`2.4.255-SNAPSHOT`) is now wired into the on-device build — skiko + all 31 compose-multiplatform-core wasm-wasi klibs republished against it, and wart-app's final whole-world link re-lowers all IR against it via the `~/.gradle/init.d/kt-86415-stdlib-override.gradle.kts` redirect. **The fork adapter (with the `State::with` self-heal) was deliberately KEPT in this build**, not removed. Consequence for verification: a clean Tooltip/DatePicker run with this build does NOT by itself prove the stdlib fix — if the patch were ineffective, the self-heal would silently re-init State and the only signal would be the logcat line `wart fork: wasi adapter State corruption — recovered`. **Watch for that message: absent = stdlib fix proven; present = patch incomplete.** A definitive isolated test (rebuild the component with the *stock* adapter `~/wart/skiko/wasi_snapshot_preview1.reactor.wasm`, where a recurrence hard-SIGILLs) was offered but the user declined for now — so the stock-adapter verification remains outstanding. Boot + scripted interaction on 2026-05-20 showed no self-heal message and no crash, but did not precisely exercise the TooltipBox long-press path.

**File this upstream as a Kotlin/Wasm bug.** Minimum standalone repro lives at
`/home/harry/wart/kt-memalloc-repro/` — runs end-to-end with Kotlin 2.4.0-RC,
wasm-tools component new + wasmtime run. No skiko, no Compose, no Android.
Output demonstrates that every `withScopedMemoryAllocator` reuses address 0
regardless of what prior scopes wrote — including the bytes Kotlin's *own*
internal `println` buffer scope leaves behind. Build + run with:

```bash
cd /home/harry/wart/kt-memalloc-repro
./gradlew compileProductionExecutableKotlinWasmWasi
wasm-tools component new build/.../kt-memalloc-repro.wasm \
  --adapt /home/harry/wart/wasmtime-src/target/wasm32-unknown-unknown/debug/wasi_snapshot_preview1.wasm \
  -o /tmp/repro.wasm
wasmtime run --wasm gc=y --wasm function-references=y --wasm exceptions=y \
  --wasi preview2 /tmp/repro.wasm
```

**Patch proposal** (in `kotlin/wasm/unsafe/MemoryAllocation.kt`):

```kotlin
internal fun destroy() {
    destroyed = true
    parent?.let { p ->
        p.suspended = false
        // FIX: propagate child's used range so parent's next createChild()
        // doesn't reuse the same address space. Requires availableAddress
        // visibility changed from private to internal (read + write).
        if (availableAddress > p.availableAddress) {
            p.availableAddress = availableAddress
        }
    }
}
```

Caveats: this changes the semantics of `withScopedMemoryAllocator`'s "scoped"
behavior — memory used inside the block is no longer truly reclaimable from
the parent. That's intentional for canonical-ABI long-lived allocations but
breaks the existing "scopes are ephemeral" contract for plain marshalling
use cases. A nicer fix would be to give `componentModelRealloc` its own
non-scoped allocator pool that doesn't share state with `withScopedMemoryAllocator`.
