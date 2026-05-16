---
name: wasmWasi single-thread constraint is the Kotlin runtime, not WASI
description: Don't tell the user "WASI is single-threaded" or "wasmtime is single-threaded" — both are false. WASI Preview 2 has wasi:thread/spawn and wasmtime can host threaded guests. What's single-threaded in this project (verified 2026-05-13) is the Kotlin/Wasm wasmWasi guest as built with kotlin-stdlib 2.1.20 + kotlinx-coroutines-core 1.10.2: those klibs do NOT contain wasi-thread-spawn imports or thread-pool dispatchers (only LimitedDispatcher.Worker scheduler primitives + a JsMainDispatcher analogue). If Kotlin/Wasm adds threading in a future release we'd get it for free; until then, "single thread" is the Kotlin runtime layer, not the WASI/wasmtime layer.
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
**Rule.** When discussing concurrency in this project, frame it as "the wasmWasi Kotlin/Wasm runtime runs on one thread for now". Do not say "WASI is single-threaded" or "wasmtime is single-threaded".

**Why (with verification):**
- **WASI Preview 2** includes `wasi:thread/spawn` (descendant of WASI P1's wasi-threads proposal). Real OS threads, shared linear memory, atomic instructions.
- **wasmtime** supports multi-threaded guests.
- **What I checked**: unzipped `kotlinx-coroutines-core-wasm-wasi-1.10.2.klib` and `kotlin-stdlib-wasm-wasi-2.1.20.klib` in the gradle cache. Strings dump shows scheduler primitives (`LimitedDispatcher`, `tryAllocateWorker`, `obtainTaskOrDeallocateWorker`, `JsMainDispatcher`) but **no** `wasi_thread_spawn`, `newSingleThreadExecutor`, `wasi:thread/spawn`, or any OS-thread-spawn references. The "Worker" term in these klibs is the kotlinx-coroutines LimitedDispatcher.Worker — a scheduling bookkeeping entity, not an OS thread.
- Kotlin/Wasm's `wasmWasi` target as of these versions doesn't emit thread spawns. wasmJs is similarly single-threaded today; the wasm-threads proposal + wasi-threads support are tracked but not yet shipped.

What this means for our debugging:
- `WasiFrameDispatcher` IS explicitly single-threaded (we wrote it).
- `kotlinx.coroutines` on wasmWasi IS single-threaded today (verified).
- Concurrency bugs in this project are "racing coroutines on the lone Kotlin dispatcher thread", not "WASI can't do threads".

**How to apply:**
- When the user asks "is X multi-threaded": answer "the Kotlin runtime is single-threaded today; WASI and wasmtime aren't the bottleneck."
- When designing primitives that could one day be parallel: keep that path open (don't bake in single-thread assumptions in WIT/host code), but don't bother adding locks/atomics in guest Kotlin until the runtime adds threading.
- When a Kotlin version bump happens, recheck the klib strings — if `wasi_thread_spawn` appears, we have threads.
