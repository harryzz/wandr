rootProject.name = "wandr-leak-repro"

// Standalone build — consumes everything from mavenLocal. Mirrors
// wandr-app's pattern. We deliberately depend on ONLY skiko-wasm-wasi
// and kotlinx-coroutines-core; no Compose runtime, no Compose UI,
// no Material3. The point of this project is the leak-bisect from
// task 24's step 1: minimal Kotlin/Wasm + coroutines to see whether
// the WasmGC-heap leak observed in task 23 reproduces below Compose.
