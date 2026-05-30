---
name: wasmWasi reallocAllocator state pollution
description: Kotlin/Wasm 2.4.0-Beta2 wasiRandomGet leaves reallocAllocator non-null; defensive freeAll required at start of every WIT import
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
On wasmWasi component-model + wasi adapter setup, the FIRST WASI call (e.g., `kotlin.random.Random.nextInt()` triggering `wasiRandomGet`) causes the WASI adapter's `State::new` to invoke our main module's `cabi_realloc`. That sets `kotlin.wasm.unsafe.reallocAllocator` non-null. Stdlib's `wasiRandomGet` does NOT call `freeAllComponentModelReallocAllocatedMemory()` on exit, so the global stays polluted. The next `withScopedMemoryAllocator { ... }` call hits `createAllocatorInTheNewScope` which checks `reallocAllocator == null` and throws `IllegalStateException("Can't create new allocators while realloc-allocated memory is not freed")`. The exception is a normal Kotlin tag-0 throw and IS catchable by `try_table catch_all_ref`.

**Why:** Compose initial composition uses `Random.Default` for hash seeds — first access triggers wasiRandomGet which pollutes reallocAllocator. Subsequent withScopedMemoryAllocator inside Compose throws. Since Compose's internal try/finally lowerings catch this and re-throw via throw_ref, eventually the exception escapes back to host as `wasmtime::ThrownException` ("thrown Wasm exception").

**How to apply:** In all generated WIT bindings (`SkikoUi.kt` `Canvas.Import.*` overrides), add `freeAllComponentModelReallocAllocatedMemory()` as the FIRST line of every override, before `withScopedMemoryAllocator { ... }`. Pattern:

```kotlin
override fun foo(...) {
    freeAllComponentModelReallocAllocatedMemory()       // <-- add this
    withScopedMemoryAllocator { allocator ->
        ... WIT call ...
        freeAllComponentModelReallocAllocatedMemory()   // already there
    }
}
```

The leading freeAll is a no-op when reallocAllocator is already null (cheap), and clears it when polluted. Safe everywhere.

**Still needed even after [[kotlin-wasm-scopedmemory-destroy-bug]] is fixed upstream:** the two issues compose — `freeAll` is what *triggers* the buggy `destroy()`. Removing the freeAll would dodge that destroy chain but immediately hit `IllegalStateException` from `createAllocatorInTheNewScope`'s `check(reallocAllocator == null)`. To drop the freeAll workaround you'd need a separate upstream change letting `withScopedMemoryAllocator` suspend/resume an active `reallocAllocator` instead of refusing to nest — bigger than KT-86415. So in practice: keep the freeAll forever; KT-86415's patch just makes the freeAll's downstream `destroy()` call no longer leak State's range.

**Diagnostic for similar issues:** Use `Store::take_pending_exception()` in the host (wasmtime ≥44 with `gc` + `exceptions` features) and walk the Throwable struct: field 0 of exnref → AnyRef → struct → field 4 (message: String) → field 5 (length) + field 6 (chars: WasmCharArray of i16 UTF-16). See host/src/lib.rs ~line 188.
