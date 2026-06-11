# K/Wasm: `onExportedFunctionExit` fires inside `cabi_realloc`, running user code while canonical-ABI realloc memory is pending

*Draft bug report, prepared 2026-06-11 (wandr project). Ready to file
against the Kotlin issue tracker; sibling of
[KT-86415](https://youtrack.jetbrains.com/issue/KT-86415).*

## Summary

The Kotlin/Wasm compiler wraps **every** `@WasmExport` function in an
epilogue that calls `kotlin.wasm.internal.invokeOnExportedFunctionExit()`
when the outermost export call returns. The function exported as
`cabi_realloc` (per the Component Model canonical ABI) is itself an
`@WasmExport` — so the hook also fires there.

That is the one export where running arbitrary user code is unsafe by
construction: the **host** calls `cabi_realloc` while lowering arguments
for an upcoming export call (e.g. a record containing `string`s). When
`cabi_realloc` returns, the allocated memory is *pending* — the host is
about to write argument bytes into it and then invoke the export that
lifts them. `kotlinx-coroutines` registers its event-loop pump on
`onExportedFunctionExit`; if any coroutine jobs are queued (in a real
application they almost always are), the pump runs user code in this
window. Two failure modes follow:

1. The pumped code calls `withScopedMemoryAllocator` →
   `createAllocatorInTheNewScope` →
   **`IllegalStateException("Can't create new allocators while
   realloc-allocated memory is not freed")`**. The exception propagates
   out of `cabi_realloc` into the embedder as an unhandled Wasm
   exception. Under wasmtime this **poisons the component instance**:
   every subsequent call fails with `wasm trap: cannot enter component
   instance`, killing the application.
2. Even if nothing throws, pumped code that calls
   `freeAllComponentModelReallocAllocatedMemory()` (the documented
   discipline for binding code) or allocates over the freed region
   **corrupts the in-flight argument bytes** before the export lifts
   them — silent data corruption.

No user-level code can intercept this: the throw happens in the
compiler-generated epilogue, outside the `cabi_realloc` body, so a
`try/catch (t: Throwable)` inside the user's `cabi_realloc`
implementation never sees it. (Verified empirically — see Diagnosis.)

## Environment

- Kotlin 2.4.0-RC, `wasm-wasi` target (also reproduces against current
  master stdlib sources by inspection — the epilogue and hook are
  unchanged).
- Component built with `wasm-tools component embed` + `component new
  --adapt wasi_snapshot_preview1.wasm` (the same pipeline as
  `Kotlin/sample-wasi-http-kotlin`).
- Embedder: wasmtime 45, host calling a guest export whose WIT signature
  carries `string`s (host→guest lowering calls the guest's
  `cabi_realloc`).
- kotlinx-coroutines in the guest with at least one queued job at call
  time (e.g. a UI frame dispatcher; any `launch {}` whose continuation
  is parked).

## Reproduction sketch

1. Kotlin/Wasm module exporting, per the canonical ABI:
   - `cabi_realloc` → `componentModelRealloc` (the standard one-liner);
   - a WIT export taking a record with `string` fields, e.g.
     `on-key: func(ev: key-event)` where
     `key-event = record { code: string, text: string, … }`.
2. Inside the guest, start a coroutine that suspends and re-queues
   continuously (a frame loop / `Dispatchers` job) so the
   kotlinx-coroutines event loop registered on `onExportedFunctionExit`
   always has work.
3. From the host (wasmtime bindgen), call the export with non-empty
   strings.
4. The host's lowering calls the guest's `cabi_realloc`; its epilogue
   pumps the queued job; the job touches `withScopedMemoryAllocator`;
   `IllegalStateException` escapes as `thrown Wasm exception`; the next
   call into the instance traps `cannot enter component instance`.

A guest **without** queued coroutine jobs does not reproduce — which is
exactly why simple samples and clean-room tests pass while real
applications fail on the first such call.

## Diagnosis trail (how this was pinned down)

- A clean-room guest (same WIT record-with-strings, same stdlib +
  adapter, no coroutines) survives 100,000 randomized host→guest calls
  on both JIT x86-64 and AOT aarch64. A live Compose-based application
  fails on the **first** call.
- The failure persists with an **empty** export body and with
  `cabi_realloc` wrapped in `catch (t: Throwable)` — the catch provably
  never runs (we converted each known exception message into a distinct
  wasm *trap flavor* — out-of-bounds load vs. stack exhaustion vs.
  rethrow — as a side channel; only the rethrow flavor ever surfaced,
  i.e. the inner code never threw).
- Disassembling the component (`wasm-tools print`) shows the generated
  wrapper around the user's `cabi_realloc`:

  ```wat
  (func $...cabi_realloc (param i32 i32 i32 i32) (result i32)
    global.get $kotlin.wasm.internal.isNotFirstWasmExportCall
    local.set $~currentIsNotFirstWasmExportCall
    ... try_table (catch_all_ref ...)
        i32.const 1
        global.set $kotlin.wasm.internal.isNotFirstWasmExportCall
        ... call $kotlin.wasm.unsafe.componentModelRealloc ...
    ;; epilogue, on BOTH normal and exceptional exit:
    local.get $~currentIsNotFirstWasmExportCall
    i32.eqz
    if
      call $kotlin.wasm.internal.invokeOnExportedFunctionExit  ;; ← the pump
    end
    ...)
  ```

  Since the host calls `cabi_realloc` directly (not nested inside
  another Kotlin export), `isNotFirstWasmExportCall` is false and the
  hook fires every time.

## Suggested fix

Do not run the `onExportedFunctionExit` callback while canonical-ABI
realloc memory is pending. One-line guard in
`libraries/stdlib/wasm/wasi/src/kotlin/internal/internalCallback.kt`:

```kotlin
internal fun invokeOnExportedFunctionExit() {
    // cabi_realloc is itself an exported function: its exit happens
    // mid-lowering, after the host allocated argument memory but before
    // the export that consumes it runs. Running user code here either
    // throws ("Can't create new allocators while realloc-allocated
    // memory is not freed") or corrupts the pending arguments. Defer
    // the callback to the next normal export exit.
    if (kotlin.wasm.unsafe.isComponentModelReallocPending()) return
    @OptIn(InternalWasmApi::class)
    onExportedFunctionExit?.invoke()
}
```

with a trivial accessor next to `componentModelRealloc` in
`MemoryAllocation.kt`:

```kotlin
internal fun isComponentModelReallocPending(): Boolean =
    reallocAllocator != null
```

Deferral is safe: the export call that consumes the lowered arguments
runs immediately afterwards, and *its* exit (allocator state clean)
pumps the queued work. We have been running this patch in production;
it resolves the failure with no observed side effects, and host→guest
record-with-strings calls now pass the same 100k-iteration stress on
JIT and AOT with a coroutine-heavy guest.

An alternative (or complementary) fix is for the compiler to omit the
`invokeOnExportedFunctionExit` epilogue specifically for the
`cabi_realloc` export, but the stdlib guard also protects any other
allocator-sensitive export and keeps the behavior data-driven.

## Relationship to KT-86415

KT-86415 is about `realloc`-allocated memory being freed/reused out from
under a long-lived holder (use-after-free). This issue is the
*scheduling* sibling: the runtime actively runs user code in the window
where the canonical ABI requires realloc'd memory to stay untouched.
Both stem from the same un-modeled invariant: **between a host's
`cabi_realloc` call and the subsequent export invocation, the guest's
linear-memory allocator state must not be observed or mutated by user
code.** Any robust fix for component-model support in Kotlin/Wasm needs
to encode that invariant; this report covers the half that affects every
consumer of the official wit-bindgen flow the moment they combine
kotlinx-coroutines with host-lowered export arguments.
