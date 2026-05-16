---
name: PathBuilder must mirror SkiaBackedPath mutatePath calls
description: Compose-ui's SkiaBackedPath calls bare method names inside `mutatePath { ... }` (a PathBuilder.()->Unit lambda); our shim PathBuilder must provide every such name or unqualified calls fall back to the enclosing SkiaBackedPath overrides and infinite-recurse → wasm stack overflow → SIGSEGV in wasmtime engine_type_index.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
When implementing the wasmWasi `org.jetbrains.skia.PathBuilder` shim, it MUST expose every method that upstream compose-ui's `SkiaBackedPath.skiko.kt` calls inside its `mutatePath { ... }` lambdas. The lambda has type `PathBuilder.() -> Unit`; an unqualified call inside it resolves first against PathBuilder, but if absent, falls through to the *outer* receiver — which is SkiaBackedPath itself. SkiaBackedPath has overrides for the same names (reset, close, addRoundRect, etc.) — so the call recurses into itself.

Caught in May 2026: `SliderDefaults.Track` crashed with SIGSEGV in `wasmtime::runtime::vm::instance::Instance::engine_type_index` / `get_interned_func_ref` whenever a Slider was composed. Root cause was `SkiaBackedPath.reset()` doing `mutatePath { reset(); setFillType(fillMode) }` — and our PathBuilder had no `reset()` method, so the unqualified `reset()` resolved to `SkiaBackedPath.reset()` itself, infinite-recursing → stack overflow → wasmtime trap.

**Why:** Stack-overflow traps in wasm-gc/function-references mode surface as SIGSEGV in `engine_type_index`/`get_interned_func_ref` because the indirect-call type-check chases its operand off the bottom of the wasm stack. The crash trace gives no Kotlin stack frames at all (only 4 native frames recovered) — so this looks like a "wasmtime codegen bug" rather than what it really is (Kotlin source-level infinite recursion).

**How to apply:**
- When adding methods to compose-ui-graphics's SkiaBackedPath path of the shim layer, cross-check each `mutatePath { fooMethod(...) }` in upstream `SkiaBackedPath.skiko.kt` against the methods defined on our `PathBuilder`.
- If a SIGSEGV in `engine_type_index` shows `Cause: stack pointer is not in a rw map; likely due to stack overflow`, do NOT investigate it as a wasmtime codegen / Cranelift opt-level bug. Look first for an unqualified call inside a `PathBuilder.()` (or any other extension-receiver lambda) that's silently resolving to the enclosing class method.
- When a fix to skiko `PathBuilder` doesn't take effect, remember that `compose-ui-graphics-wasi` and `skiko-wasm-wasi` are BOTH precompiled to mavenLocal. After editing skiko's Kotlin sources you must `./gradlew :publishToMavenLocal` from skiko/skiko/, then republish every Stage-3 module that *inlines* against PathBuilder (compose-ui-graphics-wasi at minimum), and only then rebuild test-app. Forgetting to republish skiko leaves the inlined `mutatePath { reset() }` bytecode-resolved against the stale PathBuilder.
