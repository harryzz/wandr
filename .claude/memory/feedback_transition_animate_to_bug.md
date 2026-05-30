---
name: identityHashCode wasi actual must use a STABLE hashCode for the lookup bucket
description: A subtle violation of the equals/hashCode contract in `compose-runtime-wasi`'s `identityHashCode` actual caused Compose's `DerivedSnapshotState` validity check to misfire, manifesting as Material3 widgets that use `updateTransition` (Checkbox, DropdownMenu, ExposedDropdownMenu, anything with a transition-driven state) toggling correctly ONCE and then sticking. Fixed by bucketing the internal HashMap by `target.hashCode()` instead of a mutating global counter.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
**Bug location:** `compose-runtime-wasi/src/wasmWasiActuals/kotlin/androidx/compose/runtime/internal/Utils.wasmWasi.kt`.

**Pre-fix code:**
```kotlin
private var nextHash = 1
private val identityMap = HashMap<Identity, Int>()

private class Identity(val target: Any) {
    override fun equals(other: Any?) = other is Identity && other.target === target
    override fun hashCode(): Int = nextHash  // ← reads MUTATING global counter
}

internal actual fun identityHashCode(instance: Any?): Int {
    if (instance == null) return 0
    val key = Identity(instance)
    val existing = identityMap[key]      // (1) lookup at bucket = nextHash NOW
    if (existing != null) return existing
    val v = nextHash++                    // (2) value to store
    identityMap[key] = v                  // (3) store at bucket = nextHash AFTER (2)
    return v
}
```

The lookup at (1) and the put at (3) use **different** bucket indices because `nextHash` was incremented at (2) between them. Worse, a SECOND call to `identityHashCode(sameObject)` constructs a new `Identity(sameObject)` whose `hashCode()` is whatever `nextHash` happens to be NOW — different from any previous time. HashMap walks an empty bucket and returns null → we PUT a new entry → repeat. The map grows linearly with calls and every call returns a fresh value.

**Why it crippled Material3 transitions:**
`DerivedState.kt::DerivedSnapshotState.readableHash()` builds a structural hash of the derived state's dependency graph using `identityHashCode(dependency)` and `identityHashCode(record)`. That hash is then compared against `resultHash` in `isValid()` to decide whether the cached derived value is fresh. With unstable identity hashes, `resultHash` could appear "valid" when dependencies had actually changed — caching stale derived values. In `Transition.animateTo`, the derived state `runFrameLoop` controls a conditional `if (runFrameLoop) { DisposableEffect { coroutineScope.launch { while (isActive) withFrameNanos { … } } } }`. Once it flips to true and the animation completes, the cache then "decides" subsequent target changes don't change the derived value → `runFrameLoop` never flips true again → the animation coroutine never re-launches.

**Fix:**
```kotlin
private class Identity(val target: Any) {
    override fun equals(other: Any?) = other is Identity && other.target === target
    override fun hashCode(): Int = target.hashCode()  // ← stable; collisions OK
}
```

`target.hashCode()` is stable for the target's lifetime (Object default is fixed-per-instance, data-class hashCode is fixed-per-content). HashMap allows collisions and uses `equals` (which is `target === target`) to disambiguate within a bucket.

**Verified 2026-05-13 on device:**
- Checkbox toggles correctly across 4 sequential taps (checked → unchecked → checked → unchecked → checked).
- All other widgets that worked before (Counter, Slider, Switch, RadioButton, Int/Bool toggle, ProgressIndicators) continue to work.
- DropdownMenu's *popup* still doesn't render — but that's the separate `ComposeSceneLayer` overlay issue (task #50), not this bug; the Button's `onClick` and the `expanded` state change correctly. With the popup wiring in place, the menu should now render.

**Things tried earlier that did NOT fix it** (left in tree as they're improvements regardless):
- Caching frame nanos in `compose-ui-wasi/UiActuals.wasi.kt::currentTimeMillis()` to avoid `org.jetbrains.skiko.currentNanoTime()` polluting the WIT realloc allocator. This is a real underlying gotcha for `RectManager.dispatchCallbacks()` and worth keeping, just wasn't this bug.
- `WasiFrameDispatcher` (queue-based) passed to `PlatformLayersComposeScene(coroutineContext = …)` to replace `Dispatchers.Unconfined` (upstream's own `// TODO: Remove Dispatchers.Unconfined as a default` flagged default). Kept; it's the right thing per upstream's own note.

**How to apply:** if you write any other `expect/actual` for the wasi target that needs identity semantics (WeakReference, WeakHashMap, identity-keyed caches), use the same pattern — bucket by `target.hashCode()` and disambiguate equality with `===`. NEVER bucket by a value that mutates over time.
