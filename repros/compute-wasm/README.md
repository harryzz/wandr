# compute-wasm — OpenAttributeGraph (jcmosc/Compute) running on `wasm32-wasip1`

Persisted snapshot of the work that got **Apple's AttributeGraph reimplementation
running on WASI** — the make-or-break layer under OpenSwiftUI-on-wandr. Reactive,
dependency-tracked graph computation executes under wasmtime:

```
Compute AttributeGraph on wasi: attribute.value = 42      # value attribute
Compute rule on wasi: ruleAttr.value = 42                 # a Rule whose update() runs
```

Full analysis: `docs/swift-openswiftui-wandr-feasibility.md`. This dir is the
reproducible artifact (the actual edits live in `/tmp` clones, which are ephemeral —
hence this patch). Upstream PR target: **`jcmosc/Compute`**.

## Pinned bases

| Repo | Rev | Notes |
|---|---|---|
| `github.com/jcmosc/Compute` | `86c38408` (Merge #43, release/0.3) | the engine; **patch applies here** |
| `github.com/jcmosc/swift-runtime-headers` | `626688ce` (release/6.3) | Compute submodule; **unmodified** (vendors swift + llvm/ADT headers) |
| Swift SDK | `swift-6.3.2-RELEASE_wasm` on Swift 6.3.2 | `swift sdk install` it first |

## Contents
- `compute-wasm.patch` — the 17-file diff to `jcmosc/Compute` (the whole wasm port)
- `shims/` — WASI build shims (live on the clang include path; **not** upstream):
  - `syslog.h` — stderr stub (production: lower to `wasi:logging`, which wandr has)
  - `openssl/sha.h` — header-only real SHA-1 (wasi-libc has no openssl)
  - `wasi_compat.h` — `typedef unsigned int uint;` (wasi-libc lacks the BSD alias)
- `computerun/` — the standalone test harness (`Package.swift` + `main.swift`).
  **Edit `Package.swift`**: point the `.package(path:)` dependency at your Compute clone.

## Reproduce

```bash
# 1. Clone Compute at the pinned base + submodules, apply the port
git clone https://github.com/jcmosc/Compute /tmp/Compute
cd /tmp/Compute && git checkout 86c38408 && git submodule update --init --recursive
git apply /home/harry/wandr/repros/compute-wasm/compute-wasm.patch

# 2. Build + run the harness (set computerun/Package.swift path to /tmp/Compute first)
cd /home/harry/wandr/repros/compute-wasm/computerun
SHIMS=/home/harry/wandr/repros/compute-wasm/shims
BASE=~/.swiftpm/swift-sdks/swift-6.3.2-RELEASE_wasm.artifactbundle/swift-6.3.2-RELEASE_wasm/wasm32-unknown-wasip1
swift build --swift-sdk swift-6.3.2-RELEASE_wasm \
  -Xcc -I"$SHIMS" -Xcc -fno-exceptions -Xcc -DSWIFT_INLINE_NAMESPACE=__runtime \
  -Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN -Xcc -D_WASI_EMULATED_PROCESS_CLOCKS \
  -Xlinker -L"$BASE/WASI.sdk/lib/wasm32-wasip1" \
  -Xlinker -L"$BASE/swift.xctoolchain/usr/lib/swift_static/wasi" \
  -Xlinker -L"$BASE/swift.xctoolchain/usr/lib/swift/wasi" \
  -Xlinker -lc++abi -Xlinker -lwasi-emulated-mman -Xlinker -lswiftCore
wasmtime run -W all-proposals=y .build/wasm32-unknown-wasip1/debug/computerun.wasm
```

## What the patch does (all `#if defined(__wasi__)`-guarded)

1. **Allocator** (`Table.cpp`) — anonymous `mmap(MAP_PRIVATE|MAP_ANON)` (wasi-emulated-mman)
   + `memcpy`-grow; `madvise` no-op; `memfd_create`/`MAP_SHARED` eliminated (WASI#304).
2. **Exceptions** — built with `-fno-exceptions` (wasi SDK has no exception runtime;
   Compute has no explicit `throw`).
3. **Demangle** — `-DSWIFT_INLINE_NAMESPACE=__runtime` (the lib defines the symbol in
   `Demangle::__runtime::`; headers default to plain `Demangle::`).
4. **The `@_silgen_name` → C-import rule** (the core finding): on wasm,
   `@_silgen_name` lowers a call with the *Swift* CC, whose wasm signature mismatches
   clang's C ABI (`signature_mismatch` at `call_indirect`). A header-declared,
   C-imported entry called with the C ABI works (proven by a minimal control). Applied to:
   - `IAGGraphInternAttributeTypeC` (synchronous closure — invoked in-language via a context ptr)
   - `IAGRetainClosureC` + a Swift `_UpdateBox` + a `@convention(c)` trampoline
     (**stored** closure — C++ calls it later via C ABI; `_update` made plain-C on wasm)
   - `IAGGraphSetOutputValueC` (the rule's output write)
5. **ABI asserts** — 64-bit-layout `static_assert`s (Apple binary-compat) neutralized
   for wasm32. Misc: `uint` typedef, `print_cycle` (Apple `.mm`) guard, `.wasi`
   Package conditions, link libs.

## Scope / status
- ✅ value attribute and ✅ a computed **Rule** (update runs, reads a dependency, writes output) both yield 42.
- The same C-import conversion applies mechanically to the remaining ~15
  `@_silgen_name` sites as a full OpenSwiftUI app reaches them.

## Before upstreaming to `jcmosc/Compute` (cleanup for a real PR)
- Turn the **commented-out** ABI `static_assert`s into `#if __POINTER_WIDTH__ == 64`
  guards (don't delete them).
- Fold the build flags + shims into `Package.swift` (`cxxSettings`/`linkerSettings`
  `.when(platforms: [.wasi])`) so a plain `swift build --swift-sdk` works.
- Replace the `syslog` stderr stub with a `wasi:logging` lowering for production.
