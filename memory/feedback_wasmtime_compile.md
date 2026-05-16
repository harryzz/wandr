---
name: wasmtime AOT compile flags
description: Required wasmtime compile flags for Kotlin/WASM component targeting aarch64-linux-android
type: feedback
originSessionId: fdc1d8b3-20a9-450b-aa4a-1c43a421f4ff
---
Always use these flags when AOT-compiling the Kotlin-generated component for Android:

```bash
wasmtime compile --target aarch64-linux-android \
  --wasm component-model --wasm gc --wasm function-references --wasm exceptions \
  -o skiko-component.cwasm skiko-component.wasm
```

**Why:** Kotlin/WASM output uses GC types (typed arrays), function references (non-nullable types), and exceptions. Without these flags, wasmtime fails with:
- "array indexed types not supported without the gc feature"
- "function references required for non-nullable types"
- "exceptions proposal not enabled"

**How to apply:** Use this exact set of `--wasm` flags for every AOT compile step in this project.
