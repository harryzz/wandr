# Compute → wasm32-wasip1 port log

# ═══ #12 OPTION (c) "complete teardown edge-cleaning" — DISPROVEN by measurement (2026-06-29) ═══
User chose (c). Built two teardown AUDITS that run at the end of invalidate_now, just before
`delete removed_subgraph`, on wasm:
  - SUBGRAPH audit: any survivor whose _parents/_children still points at a dying subgraph? -> 0.
  - NODE audit: any survivor node whose output_edges still targets a node in a dying subgraph? -> 0.
Both ZERO across the whole demo run. PLUS oagoffset case3 (generic dependent teardown) passes. =>
Compute's graph-internal teardown ALREADY removes every cross-subgraph reference it owns; there is NO
edge-cleaning gap to fix. (c)'s premise is false.
Also tried the one concrete asymmetry found by reading — add_input_dependencies registers the reverse
output edge via resolve(SkipMutableReference) [no weak] but remove_removed_input removed it via
resolve(SkipMutableReference|EvaluateWeakReferences) [with weak]; matching them (drop EvaluateWeak in
removal) did NOT fix the crash -> reverted (unproven necessity).
Yet propagate_dirty(asyncSignal) still aborts at frame #14 in Node::state() (a pure field read -> the
abort is data::ptr operator->'s `_offset!=0` assert / an offset that is < table max but beyond committed
wasm memory). pd2 proved the output edges are NOT >= table-max ("wild"), so the target is a recycled/stale
offset, not an out-of-range one. CONCLUSION: the dangling ref is NOT a graph edge — it is either a
CONSUMER-held Attribute handle (OpenSwiftUI's makeItem invalidates an asyncSignal whose node was torn down)
or an output_edges entry aliased by page-recycle AFTER teardown. Teardown cannot own either. So the fix
converges (a 3rd independent time) on OPTION (a): make wasm page-recycle SAFE — defer reclamation of a
torn-down subgraph's pages until no reference (consumer handle OR internal) can alias them. This also
matches AG's effective behavior on 64-bit Apple (pages not recycled -> every stale ref reads intact).
KEPT (verified, suite 15/15 wasm+linux): #1 input_value_ref_slow expired-weak read; #2/#3 resolve_slow
dependency guard; detector oagoffset (cases 1-3). All (c) audits/probes + the asymmetry change REVERTED.
RESUME: implement (a) — bounded page-recycle deferral (a freed subgraph's pages quarantined until end of
the render / a generation epoch), proven with an aggressive-recycle churn detector; NOT an unbounded leak.


Goal: compile clean upstream `jcmosc/Compute` (cloned at `./Compute`, HEAD `c0cc862`) for
`wasm32-wasip1` using `swift-6.3.2-RELEASE_wasm`. Every change is `#if defined(__wasi__)`-gated
with **zero behavior change on Apple/Linux**. Test harness lives in `./vw-baseline` (lib not
modified there).

Baselines already established:
- **Linux x86_64:** library compiles clean (0 errors). Value-witness for managed values is correct
  (`inits=1 deinits=1`, no leak/double-free). Foreign-ref ARC store-into-class-field works (separate repro).
- **Mechanism note:** the Apple `.mm` files (`Graph.mm`, `IAGGraph.mm`, `IAGDescription.mm`) compile to
  **empty `.o`** off-Apple — their whole body is `#if TARGET_OS_MAC`-gated. `Graph.cpp`/`IAGGraph.cpp`
  carry the core logic. Only `print_cycle` (inside the gate) is referenced by core code → undefined at
  *executable* link → stubbed in the consumer package, not the lib.

Build command (wasm):
```
cd Compute && swift build --product Compute --swift-sdk swift-6.3.2-RELEASE_wasm \
  -Xswiftc -enable-experimental-feature -Xswiftc Extern
```

---

## Step 1 — `syslog.h` guard (Platform/log.h)
**Why:** wasi-libc has no `<syslog.h>`. The non-Apple branch `#include`s it, but `log.c` logs via
`vasprintf`+`stdio` and never calls syslog, so the include is vestigial on wasi.

**Change:** `Sources/Platform/include/platform/log.h` — add a `#elif defined(__wasi__)` branch that
skips the include (Apple/Linux untouched).

**Applied (revised to shim approach, per fork):** lib source kept **pristine** (reverted the log.h
edit). Created `./wasi-shims/` with 4 header-only `static inline` shims reused from the fork:
`syslog.h` (→stderr), `openssl/sha.h` (real SHA-1), `wasi_compat.h` (`uint`), `dispatch/dispatch.h`
(`dispatch_once`). Build passes `-Xcc -I ./wasi-shims`.

**Result:** ✅ gaps 1 (syslog) + 2 (SHA1) cleared. Build advanced from `Platform` into `ComputeCxx`,
then hit the next gap (Step 2).

---

## Step 2 — `Package.swift` `.wasi` conditions for SwiftCorelibsCoreFoundation
**Why:** ComputeCxx/Utilities `#include <SwiftCorelibsCoreFoundation/CF*.h>` (CFRuntime/CFString/CFData).
That dependency is added only `.when(platforms: [.linux])`, so on **wasi** the CF headers aren't on the
include path → `file not found`.

**Change (PENDING APPROVAL):** in `Compute/Package.swift`, change the `SwiftCorelibsCoreFoundation`
dependency conditions from `.when(platforms: [.linux])` to `.when(platforms: [.linux, .wasi])` in both
the `Utilities` and `ComputeCxx` targets (and likely the `_GNU_SOURCE` defines).

**Applied:** `Compute/Package.swift` lines 38 + 99 — `SwiftCorelibsCoreFoundation` dependency condition
`.when(platforms: [.linux])` → `.when(platforms: [.linux, .wasi])`. (`_GNU_SOURCE` left untouched.)

**Result:** ✅ CF headers resolve; `SwiftCorelibsCoreFoundation` (header-only) compiles for wasi.
Build advanced into `Utilities`/`SwiftCorelibsCoreFoundation`, then hit the next gap (Step 3).

---

## Step 3 — wasi emulation flags (build command, not lib source)
**Why:** wasi-libc gates `<signal.h>` behind `-D_WASI_EMULATED_SIGNAL` ("wasm lacks signal support").
The lib also uses `mmap` (data zone → `_WASI_EMULATED_MMAN`) and clocks (`time.h` →
`_WASI_EMULATED_PROCESS_CLOCKS`) — the standard wasi-libc emulation set the fork used.

**Change (PENDING APPROVAL):** add to the **build command** (no lib/source change):
`-Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN -Xcc -D_WASI_EMULATED_PROCESS_CLOCKS`.
(The matching `-lwasi-emulated-*` link libs are only needed when we later link a test executable,
not for the `--product Compute` library build.)

**Applied:** build command gained `-Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN
-Xcc -D_WASI_EMULATED_PROCESS_CLOCKS`. No lib/source change.

**Result:** ✅ wasi feature gaps cleared; build advanced deep into `ComputeCxx` (Subgraph, Metadata,
TreeElement, …), then hit the next gap class (Step 4).

---

## Step 4 — guard 64-bit `static_assert(sizeof(...))` for wasm32
**Why:** 6 layout asserts hardcode the **64-bit** struct size; on wasm32 (4-byte pointers) the structs
are legitimately smaller, so the asserts fail. They are **verification only** — the code allocates with
`sizeof(...)` (compiler-computed), e.g. `Table.cpp` uses `sizeof(page)` — so the wrong constant doesn't
break anything; the assert just can't hold on a 32-bit ABI.

**Change (PENDING APPROVAL):** wrap each of these 6 asserts in `#if !defined(__wasi__) ... #endif`
(keep verifying on Apple/Linux 64-bit; skip on wasm32). Leave the 7th (`wchar_t == AttributeID`, relative).
  - `Data/Page.h:23` (page == 0x18)
  - `Graph/Tree/TreeElement.h:46` (TreeElement == 0x20)
  - `Graph/Tree/TreeValue.h:40` (TreeValue == 0x18)
  - `Subgraph/NodeCache.h:24,35` (Type == 40, Item == 32)
  - `Comparison/LayoutDescriptor.cpp:561` (swift::HeapObject == 0x10)

**HONEST CAVEAT:** disabling these means we no longer *verify* the wasm32 layout. It's safe **only if**
no code hardcodes these sizes (must use `sizeof`). Verified for `page`; the others (`TreeElement`,
`TreeValue`, `NodeCache`) need the same quick audit as a follow-up — flagging since disabled checks are
exactly what hid bugs before.

**Applied (Option B — ABI-aware, keeps verifying on wasm32):** each assert wrapped as
`sizeof(X) == (sizeof(void *) == 8 ? <64-bit> : <wasm32>)`.

### Validation table (field-by-field; only raw `*` pointers shrink 8→4)
`data::ptr<T>` and the `…ID` types are `uint32_t` offsets (4 bytes on **both** ABIs); `AttributeID`
is `uint32_t _value` (4 both). So a struct shrinks by exactly `(raw * pointers) × 4`.

| struct | raw `*` ptrs | 64-bit (= assert) | wasm32 | Δ |
|---|---|---|---|---|
| `page` | 1 — `zone*` | `8+4+4+4+2+2` = **24** (0x18) | **20** | −4 |
| `TreeElement` | 1 — `type*` | `8+4·6` = **32** (0x20) | **28** | −4 |
| `TreeValue` | 1 — `type*` | `8+4·4` = **24** (0x18) | **20** | −4 |
| `NodeCache::Type` | 4 — `type,equatable,mru,lru` | `8·4+4`→pad **40** | `4·4+4` = **20** | −20 |
| `NodeCache::Item` | 2 — `next,prev` | `8+4+4+8+8` = **32** | `8+4+4+4+4` = **24** | −8 |
| `swift::HeapObject` | 2 words (isa + refcount) | **16** (0x10) | **8** | −8 |

**Note:** Option B *caught a real arithmetic error of mine* — I first wrote `Type`'s wasm32 size as
24; the assert failed and the correct value is **20** (no tail padding on a 4-byte ABI). Exactly the
drift a disabled check would have hidden.

**Result:** ✅ all 5 size asserts pass on wasm32; `HeapObject` unchanged. Build advanced through the
zone/cache layer, then hit the next gap (Step 5).

---

## Step 5 — force-include `wasi_compat.h` for `uint`
**Why:** `Graph.cpp` uses the BSD type `uint` (from `<sys/types.h>` on Linux); wasi-libc doesn't define
it. Our shim `wasi-shims/wasi_compat.h` has `typedef unsigned int uint;`, but nothing `#include`s it.

**Change (PENDING APPROVAL):** add `-Xcc -include -Xcc wasi_compat.h` to the **build command** (no
lib/source change) — force-includes the one-line typedef into every TU. `-I wasi-shims` already resolves it.

**Applied:** `LayoutDescriptor.cpp:561` — `sizeof(::swift::HeapObject) == 0x10` →
`== 2 * sizeof(void *)` (2 words; 16 on 64-bit, 8 on wasm32). Build caught my wrong "passes" prediction.

**Result:** ✅✅ **`swift build --product Compute` for `wasm32-wasip1` SUCCEEDS** — both `ComputeCxx`
(C++) and the `Compute` Swift module compile; `Compute.swiftmodule` (228 KB) emitted. No errors.

---

## ✅ MILESTONE: clean upstream Compute compiles to wasm32-wasip1

**Reproducible build command:**
```
cd Compute
swift build --product Compute --swift-sdk swift-6.3.2-RELEASE_wasm \
  -Xcc -I ../wasi-shims  -Xcc -include -Xcc wasi_compat.h \
  -Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN -Xcc -D_WASI_EMULATED_PROCESS_CLOCKS \
  -Xswiftc -enable-experimental-feature -Xswiftc Extern
```

**Total changes — deliberately minimal:**

*Lib source (6 lines of real change across 6 files):*
- `Package.swift` — `SwiftCorelibsCoreFoundation` dep `.when(platforms:[.linux])` → `[.linux, .wasi]` (×2 lines)
- `Data/Page.h`, `Graph/Tree/TreeElement.h`, `Graph/Tree/TreeValue.h`, `Subgraph/NodeCache.h` (×2),
  `Comparison/LayoutDescriptor.cpp` — ABI-aware `static_assert(sizeof(...))` (Option B, still verifies on wasm32)

*Our tree (no lib source touched):*
- `wasi-shims/` — 4 header-only shims (`syslog.h`→stderr, `openssl/sha.h`=real SHA-1, `wasi_compat.h`=`uint`, `dispatch/dispatch.h`=`dispatch_once`)
- build flags: `-I wasi-shims`, `-include wasi_compat.h`, the three `_WASI_EMULATED_*` defines, `Extern`

*(The `Tests/*` `import Glibc` lines in the diff are from the earlier Linux-baseline run, not the wasm port.)*

**Next:** link a wasm **test executable** against this lib (needs the `-lwasi-emulated-*` link libs +
the `print_cycle` stub, as in the Linux test) and run the value-witness baseline **on wasm** — the
comparison that isolates whether the bug is wasm-32-bit-specific.



---

## Step 7 — link-stage build flags (no lib change)
**Why:** at link, 3 undefined symbols. Two are build flags:
- `__cxa_throw`/`__cxa_allocate_exception` → `-Xcc -fno-exceptions` (exceptions→abort; standard wasm).
- `swift::Demangle::makeSymbolicMangledNameStringRef` absent → the wasm runtime puts it in the
  `__runtime` inline namespace; `-Xcc -DSWIFT_INLINE_NAMESPACE=__runtime` makes ComputeCxx reference
  the symbol that actually exists.

**Applied:** build command gained `-Xcc -fno-exceptions -Xcc -DSWIFT_INLINE_NAMESPACE=__runtime`.
Test-executable link also adds `-Xlinker -lwasi-emulated-signal -mman -process-clocks` and depends on
the `CStubs` target (no-op `print_cycle`, the Apple-`.mm`-only symbol).

## Step 8 — `Table.cpp` wasi data-zone path (lib change)
**Why:** `memfd_create`/`madvise` are Linux-only; `vm_remap`/`MAP_SHARED` unavailable on wasi.
**Applied (4 `#if defined(__wasi__)` gated edits, mirrors the fork):**
- create: `MAP_PRIVATE|MAP_ANON` + `memset` (wasi mmap isn't zeroed); no `memfd`/`ftruncate`.
- `~table`: don't `close(_vm_region_fd)`.
- grow: fresh bigger anonymous region + **`memcpy` old→new** (no `vm_remap`/`memfd`).
- skip `madvise`.

**⚠ Bug-hunt note:** Linux uses `memfd`+`MAP_SHARED` so old/new zone mappings share the *same physical
memory*. The wasi grow `memcpy`s into a **separate** region and keeps the old one in `_remapped_regions`
→ **two divergent copies after a grow**. If anything reads via a stale OLD-region pointer while writes
go to the NEW region (or vice-versa), that's exactly a float-as-pointer-class corruption. Flagged for
the value-witness investigation.

## ✅✅ vw-baseline links + runs on wasm32 — traps in attribute interning
`vwbaseline.wasm` (66.5 MB) builds and **runs under wasmtime**, but traps:
`signature_mismatch: IAGGraphInternAttributeType` (in `Attribute(value:)` → `typeIndex(...)`).
This is a **wasm `call_indirect` type mismatch** (a C callback/function-pointer invoked with a
signature that doesn't match its definition) — a wasm-ABI porting gap, *not* the value-witness bug; it
traps before the value-witness path runs. **Next:** resolve the signature mismatch, then the test reaches
the actual value-witness comparison.


## Step 9 — thunk #1: `IAGGraphInternAttributeTypeC` (lib change, wasm-gated)
**Applied (5 gated pieces; Apple/Linux paths preserved):**
- `Graph.swift`: `#if arch(wasm32)` route `internAttributeType` through the plain-C variant with a
  non-capturing `@convention(c)` thunk (closure passed by stack pointer); `#else` keeps `@_silgen_name`.
- `IAGGraph.h`/`.cpp`: `#if defined(__wasi__)` `IAGGraphInternAttributeTypeC` (plain-C callback) decl+impl.
- `Graph.h`/`.cpp`: `#if defined(__wasi__)` `Graph::intern_type_c` (same logic, plain-C callback).

**Result:** ✅ `internAttributeType` no longer traps. The trap **moved to the next swiftcall site**:
`signature_mismatch: IAGRetainClosure` in `IAGAttributeType.init(...update:)` — i.e. the `update`
closure. (Confirms the iterative thunk class; Step 10 next.)


## Step 10 — thunk #2: `IAGRetainClosure` (lib change, wasm-gated)
**Applied (4 gated pieces):**
- `IAGClosure.h`/`.cpp`: `#if defined(__wasi__)` `IAGRetainClosureC` (plain-C variant, same swift_retain semantics).
- `AttributeType.swift`: `#if arch(wasm32)` box the `update` closure (`_UpdateBox` heap object) + a
  non-capturing `@convention(c)` `_updateTrampoline`, call `IAGRetainClosureC`; `#else` keeps `@_silgen_name`.

**Result:** ✅ no more signature traps — the test RUNS to completion on wasm32.

---

# ✅✅✅ MILESTONE 2: value-witness baseline RUNS on wasm32 — identical to Linux

```
wasmtime run -W max-wasm-stack=8388608 vwbaseline.wasm
[after-create]          alive=1 inits=1 deinits=0
[after-temp-scope]      alive=1 inits=1 deinits=0   <- graph RETAINED the managed value via value-witness
[after-subgraph-scope]  alive=0 inits=1 deinits=1   <- graph DESTROYED -> released it; deinit fired once
[after-graph (final)]   alive=0 inits=1 deinits=1   <- perfect: 1 init, 1 deinit, no leak/double-free
```

**Identical to the Linux x86_64 baseline.** So the Compute graph's value-witness / managed-reference
handling is **correct on wasm32**, not just Linux. Combined with the earlier findings (foreign-ref ARC
correct; basic value-witness correct), the OpenSwiftUI bug is **not** a basic value-witness-on-wasm32 problem.

**Full wasm build command (clean upstream + this port):**
```
swift build --swift-sdk swift-6.3.2-RELEASE_wasm \
  -Xcc -I ../wasi-shims -Xcc -include -Xcc wasi_compat.h \
  -Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN -Xcc -D_WASI_EMULATED_PROCESS_CLOCKS \
  -Xcc -fno-exceptions -Xcc -DSWIFT_INLINE_NAMESPACE=__runtime \
  -Xswiftc -enable-experimental-feature -Xswiftc Extern \
  -Xlinker -lwasi-emulated-signal -Xlinker -lwasi-emulated-mman -Xlinker -lwasi-emulated-process-clocks
```

**NOT yet tested:** the **zone-growth / node-recycling** path (Step 8's wasi grow does `memcpy` into a
divergent region — the flagged suspect). The basic create/destroy doesn't force a grow. **Next escalation:**
allocate enough attributes to force `grow_region()` + hold a value across it, and see if value-witness
still holds — that targets the actual suspected corruption.


---

# Zone-growth escalation (test + result)

**Test** (`vw-baseline/Sources/vwgrow/main.swift`, separate target; env-gated `[zone grow]` log added
to `Table.cpp::grow_region`, opt-in via `IAG_LOG_GROW=1`):
1. Store a managed value `Wrapper(tracker: Tracker(), magic: 0xABCDEF)` in an attribute (in the
   initial 1 MB zone region).
2. Allocate 80,000 attributes to push the zone past 1 MB and force `grow_region()` (on wasi this
   `memcpy`s into a fresh, divergent region — the Step-8 suspect).
3. Read the victim **across the grow**: assert `magic == 0xABCDEF` (corruption sentinel) and deref
   `tracker` (UAF sentinel).
4. Destroy subgraph/graph; check `inits == deinits`, no leak/double-free.

**Result — IDENTICAL on both ABIs:**

| | grow fired | victim across grow | refcount |
|---|---|---|---|
| **Linux x86_64** | `1048576 -> 4194304` ✅ | INTACT (magic ok, tracker valid) | `inits=1 deinits=1` ✅ |
| **wasm32-wasip1** | `1048576 -> 4194304` ✅ | INTACT (magic ok, tracker valid) | `inits=1 deinits=1` ✅ |

**Conclusion:** the wasi divergent-copy grow is **benign** for a managed value held across it — read
resolves via the offset-based `ptr<T>` to the new region, the old copy is never destroyed (no
double-free), nothing reads it stale (no UAF). So **the clean upstream Compute's value-witness AND
zone-growth handling are correct on wasm32**, same as Linux.

**Implication for the OpenSwiftUI bug:** it is **not** in the clean upstream's value-witness / zone /
grow machinery (all proven sound on wasm32). It was specific to **our fork's `IAG_SWIFT_SHARED_REFERENCE`
Subgraph** addition — which the clean upstream does **not** have (it uses unmanaged `IAG_BRIDGED_TYPE`).
The clean baseline therefore can't reproduce that bug, because the bug lived in the foreign-ref layer we
added, not in Compute itself.


---

## Steps 11-12 — rule-update path thunks (lib changes, wasm-gated)
Validating the dataflow engine (`vw-baseline/Sources/vwdataflow`) surfaced two more wasm-ABI gaps in
the **rule recompute** path (value attributes had no update, so they didn't hit these):
- **Step 11 — `AttributeType._update` field type** (`AttributeType.h`): on wasi, type it plain-C
  `void(*)(void*, IAGAttribute, void*)` (not `IAG_SWIFT_CC(swift)`), so `update()` invokes the
  `@convention(c)` trampoline with a matching `call_indirect` signature.
- **Step 12 — `IAGGraphSetOutputValueC`** (thunk #3): the rule writes its result via
  `Graph.setOutputValue` (`@_silgen_name`), which mislowers on wasm. Added the plain-C variant
  (`IAGGraph.h`/`.cpp`) + gated `Graph.setOutputValue` to use it on wasm.

(Remaining un-hit fork thunk: `IAGSubgraphApplyC` — `subgraph.apply`; not exercised by these tests.)

# ✅ DATAFLOW VALIDATION — Compute core engine, both ABIs

Suite (`vwdataflow`): basic rules, `@Attribute` dependencies, **update propagation**, multi-input
rules, multi-level chains, fan-out, String values, a 100-deep chain, and a **non-trivial intermediate**
(`Pair{String,[Int]}`) whose change must propagate through `compare_values`.

| | result |
|---|---|
| **Linux x86_64** | ALL 18 checks PASS |
| **wasm32-wasip1** | ALL 18 checks PASS (identical) |

**Conclusion:** Compute's dataflow engine — rules, dependency tracking, change propagation (incl.
through non-trivial values), chains, and fan-out — is **correct on wasm32**, identical to Linux.

**`compare_values` note:** the fork's `LayoutDescriptor::compare` wasm32 mis-compare bug is
**type-specific** (its failures were complex OpenSwiftUI view types). It is **not** triggered by these
tests — `Pair{String,[Int]}` change-detection works correctly on wasm32 — so the structural comparison
is **left as-is** (not replaced with the fork's conservative `memcmp`). Flagged to revisit only if a
specific complex type mis-compares once OpenAttributeGraph/OpenSwiftUI is layered on.

## Confidence summary (Compute, used as-is on wasm32)
Validated correct on wasm32 (each on a minimal reproducer, identical to Linux 64-bit):
- subgraph/graph lifecycle • value-witness for managed values • foreign-ref ARC store-into-field
- zone allocation + **zone growth** (confirmed grow, value survives) • node recycling
- **full dataflow**: rules, dependencies, propagation, chains, fan-out, non-trivial values


---

# ⚠️ HEAVY 2048-PATTERN STRESS — found a wasm32 BLOCKER in subgraph-child teardown

Test `vw-baseline/Sources/vw2048`: corner tests (subgraph.apply / addChild / childCount / removeChild /
invalidate / isValid / deferred-invalidation batch) + a heavy 2048-style **tile churn** (board subgraph
with per-tile CHILD subgraphs created+destroyed every "move", 800 moves, leak counter).

| | Linux x86_64 | wasm32-wasip1 |
|---|---|---|
| 7 corner checks | PASS | first 3 pass, then **CRASH** |
| 800-move tile churn (self-contained tiles) | PASS — live subgraphs bounded at 9, **no leak** | **CRASH** |

**wasm32 crash:** invalidating a **child** subgraph faults in
`IAG::vector<Subgraph::SubgraphChild, 0, uint32_t>::data()` ← `begin()` ← `Subgraph::remove_child`
← `invalidate_and_delete_`. `_buffer` = `0x6e75724c` (garbage, far past the 39 MB linear memory →
overwritten, not a real pointer). **Out-of-bounds memory access.** Linux never faults.

**Significance:** child-subgraph create/destroy is the *core* of the 2048 tile churn (every move adds/
removes tile subgraphs) — so **clean Compute as-is would crash the 2048 demo on wasm32.** This is
exactly the class the earlier minimal tests (lifecycle / value-witness / dataflow) did NOT reach,
because none of them invalidated a *child*.

**Found + fixed along the way (Step 13 — a real but SEPARATE latent bug):** the **erase-remove idiom
is misused in 8 sites** (`Graph.cpp` ×3, `Subgraph.cpp` ×5): `v.erase(std::remove(...))` calls the
single-element `erase(pos)` (= `erase(pos, pos+1)`) instead of the range `erase(first, last)`, leaving
a stale tail + inflated `_size` whenever >1 element is removed. Fixed all 8 to range-erase. **Correct
fix, but it does NOT resolve the crash above** (same garbage `_buffer`) — so the child-vector crash is
a distinct, still-open wasm32 corruption (N=0 `vector<SubgraphChild>` / `Subgraph` zone layout/init).

**Status: Compute is NOT yet bug-free on wasm32.** Core engine (lifecycle, value-witness, zone+grow,
dataflow) is proven sound; the **child-subgraph teardown path is wasm32-broken** and must be fixed
before Compute can run the 2048 demo on wasm or before layering OpenAttributeGraph.

---

# ROOT CAUSE (proven by elimination + disassembly): `Subgraph` UAF = Apple-only CF bridging used off-Apple

## Symptom
Heavy 2048-pattern test (`vw2048`): invalidating a **child** subgraph crashes on wasm32 inside
`vector<SubgraphChild,0,uint32_t>::data()` (garbage `_buffer = 0x6e75724c`) via
`remove_child` ← `invalidate_and_delete_`. Linux passes. This is the core 2048 tile churn op.

## Elimination — it is NOT 32-bit, NOT the allocator, NOT the vector, NOT the size asserts
| test | layer | x86-64 | -m32 glibc | wasm32 |
|---|---|---|---|---|
| N=0 vector + 800-move churn (`cpp-vec-probe`) | pure C++ | ✅ | ✅ | ✅ |
| full Subgraph lifecycle: CF storage + add/remove/invalidate + churn (`vwstorage`) | pure C | ✅ | — | ✅ |
| same lifecycle (`vw2048`) | **Swift** | ✅ | — | **💥** |

The only crashing configuration is **Swift-managed `Subgraph`**. `-m32` glibc passes at `ptr=4`,
pure C on the real wasi allocator passes — so 32-bit-ness, the allocator, the vector, and the
ABI-aware asserts are all exonerated.

## Disassembly evidence (`wasm-tools print vwcorner.wasm`, `corner()`)
```
call $IAGGraphCreate / IAGSubgraphCreate ×3 / IAGSubgraphAddChild ×2 / IAGSubgraphRemoveChild
call $IAGSubgraphInvalidate ×2
call $swift_release ×4          ; graph, board, a, b  -> Swift refcount-manages each Subgraph
```

## Source linchpin (clean `IAGSubgraph.cpp`)
```cpp
finalize = [](CFTypeRef ref){ auto sg=((IAGSubgraphStorage*)ref)->subgraph;
                              if(sg){ sg->clear_object(); sg->invalidate_and_delete_(false);} };
```
So **refcount → 0 ⇒ `finalize` ⇒ `invalidate_and_delete_` frees the C++ `Subgraph` and its `_children`.**

## Root cause
`typedef struct IAG_BRIDGED_TYPE(id) IAGSubgraphStorage *IAGSubgraphRef IAG_SWIFT_NAME(Subgraph);`
`IAG_BRIDGED_TYPE(T)` = `__attribute__((objc_bridge(T)))` **only** when
`__has_feature(objc_bridge_id)` (Objective-C runtime ⇒ Apple). Off-Apple it expands to **nothing** —
swift-corelibs-CoreFoundation gates its **own** `CF_BRIDGED_TYPE` on the identical condition
(`CFBase.h:223`), so even `CFString`/`CFArray` are unbridged off-Apple. Result: on Linux/wasm Swift
refcounts a CF object with **no ownership contract**; the count reaches 0 while a parent still holds
the child as a **raw pointer** in `_children` ⇒ `finalize` frees it ⇒ use-after-free. Identical on
Linux, but **benign** there (64-bit heap doesn't reuse the freed block) and **fatal** on wasm
(32-bit heap reuses it) — which is exactly why it's context-sensitive (UAF, not a deterministic
layout bug). **Not a Swift ARC bug; not 32-bit; not the asserts** — it's Compute's Apple-only
bridging used off-platform.

## Clean fix — foreign reference types, centralized, ALL non-Apple (Linux + wasm)
`objc_bridge` has no off-Apple equivalent by design. The portable, official Swift mechanism for a
refcounted C/C++ reference type is **foreign reference types** (`swift_attr("import_reference")` +
retain/release) — already present in the header as `IAG_SWIFT_SHARED_REFERENCE`, gated on
`__has_attribute(swift_attr)` (active off-Apple). Since every bridged type is a CFTypeRef, redefine
`IAG_BRIDGED_TYPE` in the **non-Apple** arm to a foreign reference using the generic
`CFRetain`/`CFRelease` — one macro arm fixes all bridged types, no per-type code, no storage change:
```c
#define IAG_BRIDGED_TYPE(T) \
  __attribute__((swift_attr("import_reference"))) \
  __attribute__((swift_attr("retain:CFRetain"))) \
  __attribute__((swift_attr("release:CFRelease")))
```
Existing `CF_RETURNS_RETAINED` annotations supply per-function ownership. Constraints to verify by
compile: (1) `CFRetain` returns `CFTypeRef`, not the typed ref; (2) `import_reference` may need C++
interop on the `Compute` import. **Result of the experiment (2026-06-25):**

The centralized macro is principled but **does NOT yield a minimal fix**, because foreign-reference
import (`swift_attr("import_reference")`) is fundamentally a **C++-interop feature**:

1. **Macro alone, no C++ interop** — compiles, but `swift_attr` is **silently ignored** → vw2048 still
   crashes identically (`0x6e75724c`). No effect.
2. **Macro + `.interoperabilityMode(.Cxx)` on the `Compute` target:**
   - **wasm:** wasm-SDK module-cycle bug — `cyclic dependency in module 'SwiftWASILibc' ->
     std_inttypes_h -> SwiftWASILibc` (~150 errors), independent of our code.
   - **Linux:** ~103 errors — C++ interop is **incompatible with `-enable-library-evolution`**
     (`@_transparent` + C++ APIs) AND **re-imports the C enums differently** (`IAGValueOptions` is no
     longer `UInt32` → ~100 broken call sites in `Graph.swift`, `RuleContext.swift`, …).

**Conclusion:** there is no clean centralized macro fix. Adopting foreign references means **migrating
the whole `Compute` module to C++ interop** (drop library-evolution, fix ~100 enum/type-import sites,
work around the wasm-SDK `std` module cycle) — a substantial project, which is exactly the rework the
fork undertook.

**Two real paths forward:**
- **(A) Full C++-interop migration** — the principled fix; `Subgraph` becomes a proper foreign
  reference everywhere. Cost: the cascade above.
- **(B) Storage decoupling (fork's approach, now understood):** keep C-interop; on non-Apple make the
  `Subgraph` storage a plain `malloc`'d, manually-refcounted struct (NOT a swift-corelibs CF/native-
  Swift object). Then Swift's spurious `swift_release` has no native refcount to drive to 0 → no
  premature `finalize` → no UAF. Smaller, avoids C++ interop entirely. (This is why the fork's
  "plain-malloc storage" was not a hack — it sidesteps the interop migration.)

Experiment reverted; tree restored to the compiling Phase-13 baseline.


---

## Path (A) C++-interop — modulemap-cycle experiment (2026-06-25), reverted

Root cause of the wasm `cyclic dependency` (traced exactly): under C++ interop the libc++ include dir
is prepended globally, so the C libc module `SwiftWASILibc` resolves `<inttypes.h>` to libc++'s
`std_inttypes_h` wrapper, which `#include_next`s back to wasi-libc's `<inttypes.h>` (`SwiftWASILibc`)
→ `SwiftWASILibc → std_inttypes_h → SwiftWASILibc`. (libc++'s own modulemap documents anticipating
this at line 2397; C++ interop's global libc++ injection defeats the mitigation.)

**Fix attempt (in the SDK `wasi-libc.modulemap`):** `[no_undeclared_includes]` on `SwiftWASILibc` +
`use` decls for the 17 `_Builtin_*` modules.
- ✅ Cyclic-dependency errors: **150 → 0** (the fix is correct for the cycle).
- ❌ Revealed the **next** layer: libc++ `<complex>` fails under C++ interop on this SDK
  (~690 errors, "declaration of anonymous class must be a definition" in `c++/v1/complex`) — a
  separate wasm-SDK C++-interop breakage, not the cycle.

**Conclusion:** swift-6.3.2's wasm SDK has **multiple stacked C++-interop bugs** (inttypes cycle, then
libc++ `<complex>`, likely more). Path (A) for wasm is a real multi-layer toolchain project, not a
one-liner. **Reverted to the clean Phase-13 baseline.** The `[no_undeclared_includes]`+`use` modulemap
fix for the cycle is documented here for if/when path (A) is resumed (or a newer SDK is available).

---

# ✅✅✅ ACTUAL ROOT CAUSE FOUND + FIXED (2026-06-25) — uninitialized member, ONE line

The entire dual-ARC / bridging / foreign-reference / finalize-UAF narrative above was **WRONG** — it was
chasing the wrong cause. Two trace experiments found the truth:

**Experiment 1 (finalize trace, both platforms):** `board` is **never** prematurely finalized — wasm
crashes with ZERO finalizes; Linux finalizes `board` once at cleanup (the 3206 mid-run finalizes are all
`subgraph=(nil)` no-ops). → premature-finalize/UAF hypothesis **refuted**.

**Experiment 2 (crash-site trace):** at `a.invalidate()`, `a._parents = [0x6e757220, 0x2581680]`. `a`
has only ONE real parent (board=`0x2581680`); `0x6e757220` (= ASCII `" run"`) is **garbage**.
`invalidate` calls `garbage->remove_child(a)` → `0x6e757220 + 0x2c` (the `_children._buffer` offset) =
`0x6e75724c` = the exact fault address.

**Root cause:** `IAG::indirect_pointer_vector::_data` (the member backing `Subgraph::_parents`) is
declared `uintptr_t _data;` with **no initializer**, and the class uses `= default` ctor. `_data == 0`
is the "empty" state. So a freshly-`new`'d `Subgraph` has an **indeterminate `_parents`**: accidentally
0 on a fresh/zeroed heap (pure-C probe, Linux → works), stale heap garbage under allocation churn (the
Swift app on wasm → `" run"` → treated as a live parent → deref → fault).

**Fix (one line, `IndirectPointerVector.h`):** `uintptr_t _data = 0;`

**Result:** wasm `vw2048` ALL TESTS PASSED (800-move churn, no leak, exit 0); Linux unchanged; wasm
dataflow unchanged. **Not** Swift ARC, **not** CF bridging, **not** foreign references, **not**
finalize/UAF, **not** 32-bit layout, **not** the wasi allocator, **not** the erase-remove idiom — an
**uninitialized member variable** exposed by heap reuse. (The earlier "32-bit heap reuses it" wording was
directionally toward heap-reuse but the mechanism is an uninitialized read, not a use-after-free.)

Debug traces removed; the only Compute change for this bug is the one-line initializer.

---

# Intensive 5-min stress (2026-06-25) — found a 2nd uninitialized-member bug; both fixed; then clean

`vw-baseline/Sources/vwstress`: graph-per-round (max heap churn) × {subgraph tree + cross-subgraph
dataflow + propagation, 40-deep chain, managed-value canary, string dataflow}, looped under a 5-min
`timeout`, checking correctness + leak each heartbeat.

**Found bug #2 (same class):** `Graph::_main_handler` / `_main_handler_context` (Graph.h:90-91) are
declared with **no initializer** and the `Graph::Graph()` member-init list never sets them. A fresh
Graph on a churned heap → `_main_handler` = non-null garbage → `has_main_handler()` true →
`call_main_handler` invokes a garbage function pointer → wasm `uninitialized element` trap (crashed at
~iter 0). **Fix:** `= nullptr` on both.

**Result after BOTH uninit fixes (`indirect_pointer_vector::_data=0` + the two `_main_handler=nullptr`):**
`EXIT=124` (5-min timeout, no crash), **peak iter=426,000**, **0 traps / 0 failures / 0 leak**
(`tracked-live=0` throughout). All earlier suites still pass.

## The class of bug (systematic, not one-off)
jcmosc/Compute has **uninitialized members** — pointer/handle/`uintptr_t` fields declared without an
initializer whose constructor doesn't set them. Harmless on a fresh/zeroed heap (Apple/Linux test
runs), **fatal under allocation churn** (the wasm app). Two found so far:
`indirect_pointer_vector::_data`, `Graph::_main_handler(+_context)`. Recommend a **static audit** of all
Compute structs for the same pattern rather than discovering each by crash. (This class — not Swift
ARC, not CF bridging, not foreign references, not 32-bit layout — was the real issue all along.)

---

## Resolved: `invalidate()` value reclamation is CORRECT (not a 3rd bug)

Question raised: a one-graph + `invalidate()`-per-round stress leaked managed values, while graph-per-round
did not. Investigated via source + a minimal test (no Apple docs exist — AttributeGraph is private; jcmosc/
Compute's code is the reference).

**Source (`Subgraph::invalidate_now`, the immediate path of `invalidate(false)`):** destroys every node
(`node->destroy(*_graph)` = value-witness destroy → releases managed refs) then `delete removed_subgraph`
(frees the C++ Subgraph + its zone). So `invalidate()` IS designed to fully reclaim.

**Minimal test (one subgraph + one `Attribute(value: Wrap(Tracked()))` + `invalidate`, looped 22k×):**
`tracked-live=0` throughout → **`invalidate()` frees the values. Confirmed, not a bug.**

**The earlier "leak" was a test-pattern artifact**, not Compute: it combined (a) the `_main_handler`
uninit bug (garbage `has_main_handler()` → `invalidate` took the *deferred* branch, never flushed) and
(b) cross-subgraph dependencies + a particular invalidate order. With the fixes + the plain pattern,
reclamation is correct. **No third bug.**

### Final tally of real bugs this investigation found + fixed (both uninitialized members, one line each):
1. `indirect_pointer_vector::_data = 0;`  (Subgraph::_parents — OOB fault under churn)
2. `Graph::_main_handler = nullptr;` (+`_main_handler_context`)  (garbage fn-ptr → "uninitialized element" under churn)
Everything else (Swift ARC, CF bridging, foreign references, finalize/UAF, 32-bit layout, erase-remove,
invalidate-reclamation) was investigated and is NOT the cause.

---

## Static-analysis audit (2026-06-25) — found 2 MORE of the same class

Ran a custom static pass over ComputeCxx headers for the exact pattern (pointer / `uintptr_t` / function-
pointer member with no `=` initializer), then cross-checked each candidate against its constructor.
26 candidates -> most are set in their ctors (Context/UpdateStack/ExternalTrace `_graph`/`_context`),
static (zero-init), or Swift-memberwise-filled (AttributeType). **Two were genuinely uninitialized in the
`Graph` ctor — same bug class as `_main_handler`:**

3. **`Graph::_keys`** (Graph.h) — lazy-init `if (_keys == nullptr) { _keys = new KeyTable; }` + `~Graph`
   `if (_keys) delete _keys`. Garbage on a churned heap -> intern_key skips the `new` and derefs garbage /
   the dtor `delete`s a garbage pointer. **Fix:** `= nullptr`.
4. **`Graph::_trace_recorder`** (Graph.h) — read as a presence check `if (_trace_recorder)`. Garbage
   non-null -> trace paths deref it. **Fix:** `= nullptr`.

After all 4 fixes: vw2048 (wasm+Linux) ALL PASS, churn clean.

### Final bug tally — 4 uninitialized-member bugs, all one-line, all the same class:
1. `indirect_pointer_vector::_data = 0;`
2. `Graph::_main_handler = nullptr;` (+`_main_handler_context`)
3. `Graph::_keys = nullptr;`
4. `Graph::_trace_recorder = nullptr;`

**Caveat — audit is a FLOOR, not exhaustive:** the custom regex only covered raw pointers / `uintptr_t` /
inline function pointers; it MISSED `_main_handler` (a typedef'd handle) and does not scan POD flags/enums/
sizes. A full **clang-tidy `cppcoreguidelines-pro-type-member-init`** pass (tool not installed here) would
be exhaustive and is the recommended next step to close the class out completely.

---

## clang-tidy exhaustive audit (2026-06-25) — `cppcoreguidelines-pro-type-member-init`

Ran clang-tidy 19 against ComputeCxx's real compile flags (35 .cpp, header-filter=ComputeCxx).
**15 unique "constructor does not initialize" warnings.** It caught the POD flags my regex missed.

**Real (actively-constructed objects — `new`'d, so default member initializers ARE effective; flags
read in control flow). 11 fixed:**
- `Graph`: `_deferring_subgraph_invalidation=false` (read by is_deferring -> the invalidate-now/deferred
  branch), `_needs_update=false`
- `Context`: `_graph_version=0`, `_needs_update=false`, `_invalidated=false`
- `Subgraph`: `_traversal_seed=0`, `_index=0`, `_flags={}`, `_descendent_flags={}`, `_dirty_flags={}`,
  `_descendent_dirty_flags={}`

**Not fixed via initializer (zone-allocated via `alloc_bytes`+`unsafe_cast` — NO ctor runs, so a member
initializer wouldn't apply; filled post-alloc + zeroed by the wasm Table.cpp memset):** `Page`,
`Zone::bytes_info`, `TreeElement`, `TreeValue`, `NodeCache::Type/Item`, `IndirectNode::_mutable`,
`InputEdge`, `LayoutDescriptor` cache, `TreeDataElement::_sorted`, `Metadata` `_heap_buffer`. These
need creation-code review only if a specific one is suspected; the member-init fix is N/A for them.

**Verification:** wasm vw2048 + Linux vw2048 ALL PASS; churn clean (0 problems) after the 11 fixes.

### Total uninitialized-member defects found + fixed (the real root class), all one-line:
`indirect_pointer_vector::_data` · `Graph::_main_handler(+_context)` · `Graph::_keys` ·
`Graph::_trace_recorder` · `Graph::_deferring_subgraph_invalidation` · `Graph::_needs_update` ·
`Context::{_graph_version,_needs_update,_invalidated}` · `Subgraph::{_traversal_seed,_index,_flags,
_descendent_flags,_dirty_flags,_descendent_dirty_flags}`

### CORRECTION on the one-graph "leak"
Earlier called a "test artifact." Re-tested with ALL fixes: one-graph + invalidate-only STILL leaks
managed values 1/round — but ONLY with cross-subgraph dependencies (the minimal self-contained case is
`tracked-live=0`, confirming `invalidate()` frees values normally). So it is NOT an uninitialized-member
bug and NOT the `_deferring` flag; it is a cross-subgraph-dependency teardown behavior, **undetermined**
as genuine bug vs test misuse. Open, separate from this class.

---

# ✅ SETTLED: the cross-subgraph teardown "leak" is a REAL Compute bug (`&` vs `|`)

Earlier (twice) I called it a test artifact. **Wrong.** Traced it definitively.

**Mechanism (proven by tracing `is_deferring` + every writer of `_deferring_subgraph_invalidation`):**
reading a *computed* attribute (a Rule like `Doubler`, or any cross-subgraph value) creates a
`Graph::UpdateStack`. Its ctor turns deferring ON and is supposed to mark itself to turn it back OFF
on exit:
```cpp
// UpdateStack.cpp ctor, when it enables deferring:
graph->_deferring_subgraph_invalidation = true;
_options = IAGGraphUpdateOptions(_options & IAGGraphUpdateOptionsEndDeferringSubgraphInvalidationOnExit); // BUG: & clears the flag
// dtor:
if (_options & ...EndDeferring...OnExit) { _graph->_deferring_subgraph_invalidation = false; } // never true
```
It's **`&` where it must be `|`** — the reset-on-exit flag is *cleared* instead of *set*, so the dtor
never resets deferring. **After the first attribute update, `_deferring_subgraph_invalidation` is stuck
`true` forever** → every later `Subgraph::invalidate()` takes the deferred branch, which is only flushed
in the *non-deferred* branch → subgraphs + their managed values pile up = the leak.

Why it hid: self-contained **value** attributes (vw2048's tiles, the minimal test) create no
`UpdateStack` → deferring stays false → `invalidate_now` → no leak. Only **computed/cross-subgraph**
attributes trigger it. And graph-per-round reclaims via graph destruction regardless of deferring.

**Fix (UpdateStack.cpp, one char):** `_options & ...` -> `_options | ...`
**Verified:** cross-subgraph test `tracked-live=0` (was growing 1/round); both `kid` and `root` now take
`invalidate_now`; vw2048 wasm+Linux PASS; heavy churn clean.

This is the 6th distinct real bug — and the only one NOT an uninitialized member: an operator typo in the
update machinery's deferred-invalidation reset. Genuine upstream bug, not test misuse.
---

# ═══ FINAL STATE (2026-06-25) ═══

## All bugs found + fixed (every one a real Compute defect, latent on Apple/Linux, exposed by wasm)
**Uninitialized members** (garbage under heap reuse; clang-tidy `cppcoreguidelines-pro-type-member-init`):
1. `indirect_pointer_vector::_data = 0` (backs `Subgraph::_parents`) — OOB fault `0x6e75724c` *(found by crash)*
2. `Graph::_main_handler = nullptr` (+`_main_handler_context`) — `call_indirect` "uninitialized element" *(crash)*
3. `Graph::_keys = nullptr` — garbage lazy-init / `delete` of garbage *(static analysis)*
4. `Graph::_trace_recorder = nullptr` — garbage presence check *(static analysis)*
5. `Graph::_deferring_subgraph_invalidation=false`, `_needs_update=false` *(clang-tidy)*
6. `Context::{_graph_version=0,_needs_update=false,_invalidated=false}` *(clang-tidy)*
7. `Subgraph::{_traversal_seed=0,_index=0,_flags={} x4}` *(clang-tidy)*

**Logic bug:**
8. `UpdateStack` deferred-invalidation reset: `&` → `|` (flag was cleared not set → `_deferring` stuck `true`
   after the first attribute update → `invalidate()` leaked subgraphs+values on cross-subgraph/computed
   attributes). *(found by tracing; settled the "one-graph leak")*

**Idiom:**
9. `Subgraph`/`Graph` erase-remove: single-element `erase(it)` → range `erase(it, end())` (8 sites).

## Ruled OUT (investigated, NOT the cause)
Swift ARC, CF/`objc_bridge` bridging, foreign-reference types, finalize/UAF, 32-bit struct layout, the wasi
allocator, the N=0 vector, `invalidate()` value-reclamation (proven correct), `compare_values` for the
tested types. The early elaborate bridging/ARC theory was wrong; measurement + clang-tidy found the truth.

## Final verification (all green)
- wasm32 `vw2048` (800-move subgraph churn): ALL TESTS PASSED
- Linux `vw2048`: ALL TESTS PASSED
- wasm32 dataflow / value-witness / zone-growth / storage / vector suites: PASS
- 5-min heavy churn (graph-per-round, 426k rounds): 0 crashes / 0 wrong values / 0 leaks
- cross-subgraph teardown: `tracked-live=0` (was leaking 1/round)

## Published
Fork `github.com/harryzz/Compute` main force-updated to commit `abb5388`
("wasm32-wasip1 port + fix 6 latent bugs…") = fresh `jcmosc/Compute` (c0cc862) + all fixes above.
Local working tree on branch `wasm32-wasip1`. Several fixes (uninitialized members, the `&`→`|`
deferred-invalidation leak, erase-remove) are platform-agnostic upstream bugs — candidate for a
focused PR to `jcmosc/Compute`.

# ═══ RESUMED 2026-06-28 — demo regressed again; found the REAL teardown bug ═══

## Method correction (why the log went stale + cycling resumed)
The 2026-06-25 state above was abandoned: findings fragmented into `.task-state`/`RESUME.md`, and
sessions re-tried the **already-ruled-out** foreign-ref/ARC theory + **masked** symptoms (an idempotent
`Node::destroy` no-op) instead of finding real defects. GOAL re-fixed: **Compute itself bug-free** — the
OpenSwiftUI demo is a *detector*, never to be worked around. Masks reverted (idempotent destroy, debug
canaries). Keep this single log.

## Bug #10 (real, one-line) — `Subgraph::invalidate_now` pass-2 double-`vw_destroy`
`Subgraph.cpp` pass-2 destroy loop kept `previous_node` (the deferred-destroy candidate) pointing at an
already-destroyed node when the **next attribute is non-direct** (an indirect node: `get_node()`==null and
`is_nil()`==false, so neither branch runs) → the following iteration re-runs `Node::destroy`→`vw_destroy`
on freed storage. A direct node followed by N non-direct attributes is destroyed **1+N** times.
- **Proof (not guess):** the pre-fix canary showed nodes in ONE subgraph destroyed with **varying**
  multiplicity (5x, 2x, 1x). Uniform => whole-subgraph re-process; **varying => per-node** = "N following
  non-direct attrs" — exactly this loop. Confirmed: with the idempotency MASK removed (a real double-
  destroy would crash ~frame 11), the demo now sails past teardown to frame #14 with **0** teardown/
  double-free events.
- **Fix (root):** reset `previous_node = nullptr;` immediately after the in-loop destroy (mirrors pass-1,
  which already handles both node kinds). Latent on Darwin (page layout / consumers don't interleave
  indirects between direct nodes here); exposed by OpenSwiftUI indirect layout attrs (`LayoutPositionQuery`).
- **Verify:** wasm suite green (oagteardown/oagrender/oagupdate/oagsubgraph PASS); demo: 0 double-destroys.
- Platform-agnostic upstream `jcmosc/Compute` bug — PR candidate.

## Bug #11 (real, one-line, the `&`/`&&` class) — `AttributeID::resolve_slow` flag tested with `&&`
`AttributeID.cpp:76` tested the weak-ref option with LOGICAL `&&`: `if (options && EvaluateWeakReferences)`.
Every sibling flag test in the function uses BITWISE `&` (lines 56/61/65/78/89). Since
`TraversalOptions::EvaluateWeakReferences` is a nonzero constant, `options && EvaluateWeakReferences`
collapses to `(options != 0)` -> the expired-weak-source check (and its nil/precondition) fires for ANY
non-empty options, even when the caller did NOT request `EvaluateWeakReferences`.
- **Fix:** `&&` -> `&`.
- **Honesty:** does NOT fix the demo crash below — that path resolves WITH `EvaluateWeakReferences` set, so
  `&&`/`&` behave identically there. This is an independent correctness defect found by reading.
- **Verify:** full 15-target wasm suite green incl. oagweakref (no regression). Suite doesn't yet have a
  test that distinguishes the buggy path (non-weak read of an expired indirect) -> targeted test is a TODO.
- Platform-agnostic upstream `jcmosc/Compute` bug — PR candidate (same class as the 06-25 `&`->`|`).

## NEXT open bug #12 — dependent updated AFTER its source died (`invalid source attribute: 61093`, frame #14)
Root (grounded by reading): `LayoutPositionQuery` does a STRICT read (`subgraph_id==0` ->
`input_value_ref_slow` uses `AssertNotNil`, `add_input` `allow_nil=false`) of `parentPosition`, which
resolves through an INDIRECT redirect node whose WEAK source (the board's position attr) was genuinely
freed/recycled (`expired()` correct; not merely deferred). A strict read must never see a dead source ->
the dependent should have been invalidated when its source subgraph was torn down, but wasn't.
`remove_node` unlinks output edges, but a node's WEAK source is not an output edge, so the indirect
redirect that weakly references the dead attr is never notified; it survives (in the reused/surviving
subtree) with an expired source. So #12 is: when a subgraph is invalidated, dependents reachable only via
weak/indirect redirects in SURVIVING subgraphs are not invalidated/repointed. NEXT READ: indirect-node
lifecycle on reconciliation — does Compute repoint a mutable indirect's source (it has a fallback
`dependency()`), and does OpenSwiftUI re-establish `inputs.position` for a reused subtree? Determine fix
layer (Compute invalidation-propagation/indirect lifecycle vs OpenSwiftUI reuse repoint) — do NOT mask.

DEEPENED (read `Graph::remove_removed_output`, the fn `remove_node` calls per output edge): the repoint
machinery EXISTS — for a dependent INDIRECT node whose `source()==dying_attr`, it resets the source via
`output_indirect_node->modify(new_source,new_offset)`, restoring the mutable indirect's `initial_source`
if still valid, else NIL. So #12 narrows to exactly one of:
  (1) the dying source's `output_edges` do NOT include the cross-subgraph `parentPosition` indirect (it
      lives in the SURVIVING/reused subtree) -> never repointed -> stays expired; OR
  (2) it IS repointed but to NIL (no valid `initial_source`) and the dependent `LayoutPositionQuery` is
      NOT invalidated -> it recomputes and strict-reads the nil indirect -> precondition.
NEXT: confirm which, by checking (a) whether an indirect's weak source registers a reverse output edge on
its source (so teardown reaches it across subgraphs), and (b) whether `modify(...->nil)` marks the
dependent dirty/invalid. That decides the fix: register/repair the reverse edge, or invalidate dependents
on repoint-to-nil. Still a Compute-vs-OpenSwiftUI layer question — resolve by reading, do NOT mask.

ANSWERED both: (b) `IndirectNode::modify` (IndirectNode.cpp:17) ONLY sets `_source`/`_offset` — it does
NOT dirty readers. (a) reverse edges register via `add_input_dependencies` -> `resolve(SkipMutableReference)`
-> `add_output_edge` on the mutable indirect (or followed source node). CONCLUSION: the repoint DOES run
(`remove_removed_output`->`modify`), but when the WHOLE ancestor chain died the mutable indirect's
`initial_source` is also expired -> repointed to NIL; the dependent `LayoutPositionQuery` (already dirty)
recomputes and STRICT-reads the nil indirect -> precondition. #12 is therefore at the Compute<->OpenSwiftUI
boundary: OpenSwiftUI updates a reused subtree whose parent position is legitimately gone. Fix decision
needs OPENSWIFTUI RECONCILIATION analysis (does reuse re-establish `inputs.position`? should the subtree be
updated at all?) — NOT another one-line Compute defect. Resume here: read OpenSwiftUI DynamicView/list
reuse repoint of `inputs.position`. Do NOT mask, do NOT make a test pass.

STRUCTURAL CONFIRMATION (eleev TileBoardView): `GeometryReader { ZStack { ForEach(matrix.flatten(),
id: \.tile.id) { ...position().transition(.blockGenerated) } } }`. GeometryReader establishes the position
chain (GeometryReader.swift:50 `inputs.position = Attribute(LayoutPositionQuery(parentPosition: inputs.position,
localPosition: rootGeometry.origin()))`); tiles are a DynamicContainer keyed by tile.id with REUSE +
transitions. `DynamicContainer.makeItem` (DynamicContainer.swift:464) wires the item position via
`makeItemLayout` ONLY at creation; the reuse branch (updateItems ~576-582: `infoItem.item=item;
unremoveItem`) SKIPS makeItemLayout -> a reused tile keeps creation-time `parentPosition`. When an ancestor
position subgraph is torn down (tile churn / transition phase rebuild), the reused tile's `parentPosition`
is stale -> expired -> strict-read -> precondition.

TWO TESTABLE FIX CANDIDATES for #12 (each needs a build+demo verify — do NOT guess-and-claim, do NOT mask):
  CANDIDATE A (Compute): in `Graph::remove_removed_output`, after `output_indirect_node->modify(...->nil)`,
    invalidate/dirty the indirect's OUTPUT readers (currently `IndirectNode::modify` only sets _source/_offset;
    readers are not A's direct outputs so they survive reading nil). Risk: may over-invalidate; verify suite.
  CANDIDATE B (OpenSwiftUI): on reuse (DynamicContainer.updateItems reuse branch), re-establish the item's
    position wiring (re-run makeItemLayout or repoint parentPosition) to the CURRENT container inputs.position
    instead of keeping creation-time wiring. First CONFIRM whether the container's inputs.position is even
    stable across updateValue (if stable, reuse is fine and the break is higher up -> favors A).
DECIDE A vs B by: trace whether `inputs.position` (the container's own position attr) is recreated each
updateValue. Stable -> the dead ancestor is above the container -> Candidate A (Compute reader-invalidation).
Recreated -> reuse must repoint -> Candidate B (OpenSwiftUI). Read DynamicContainer.updateValue + how its
own inputs.position is produced FIRST.

PROBE RESULT (2026-06-28, `[ROOT-TEARDOWN]` in invalidate_now logging body-type names of root subgraphs
with children): the torn-down ROOT subgraph (sg=0x2f46a10, parents=0, children=1) at the crash render IS
THE CONTAINER ITSELF — nodes: `EnvironmentFetch<LayoutDirection>`, `DynamicContainerInfo<DynamicLayoutViewAdaptor>`,
`Compute._External`, `DynamicPreferenceCombiner<DisplayList.Key>`, `DynamicPreferenceCombiner<HostPreferencesKey>`,
`LayoutChildGeometries`. KEY: `LayoutChildGeometries` (the attr that PRODUCES the tiles' positions) lives in
this container subgraph -> when it's torn down, a surviving/reused tile's `LayoutPositionQuery.parentPosition`
(weak ref into LayoutChildGeometries) expires -> the crash. Exact connection established.
The probe fired EXACTLY ONCE across all 14 renders (budget 80) -> the container is NOT rebuilt per-render; it
persists renders 1-13 and is torn down at render #14 specifically (a discrete event, not churn).
FINAL QUESTION for #12: why is the `DynamicContainer`/`ForEach` torn down at render #14 but stable before?
NEXT EXPERIMENT: log DynamicContainerInfo subgraph CREATION + this teardown with a render counter (e.g.
increment a global in Graph::invalidate_subgraphs end-of-render flush) to see if a NEW DynamicContainerInfo
REPLACES it at #14 (-> ForEach identity churn / spurious reconciliation teardown = fix the identity/teardown)
or it dies without replacement (-> legitimate; then surviving tiles must be invalidated/repointed = Cand A/B).
Caveat: ROOT-TEARDOWN gate requires `!_children.empty()`; container teardowns with children already removed
would be missed — relax the gate if the once-only count is doubted. Probe still IN TREE (Subgraph.cpp,
invalidate_now top, wasm-only, budget 80) — REMOVE after diagnosis. Do NOT mask, do NOT make a test pass.

MAJOR FINDING + REDIRECT (2026-06-28, `[CONTAINER-NEW]` render-tagged probe in Graph::add_attribute):
DynamicContainerInfo nodes are CREATED at render ticks 0(build),2,6,10,14,18 — i.e. 2 NEW containers
EVERY MOVE (state change), with the old one torn down (ROOT-TEARDOWN r=15). So the `DynamicContainer`
(ForEach, holding `LayoutChildGeometries` = the tiles' position source) is RE-CREATED on every move instead
of being RECONCILED/REUSED in place. That tears down the old position source while a reused tile still
references it -> the expired-source crash. `wandrRender` confirmed the host is INCREMENTAL (renderOnce =
withoutSubgraphInvalidation{render}, host kept alive), NOT a full re-make — so this is a REUSE/CHANGE-
DETECTION decision going wrong, not a re-make.
=> By [[feedback_change_detection_test_primitive]]: "re-creates not reuses" == suspect Compute's value-
COMPARE primitive (`compare_values`/LayoutDescriptor::compare). I had been tracing the consumer (OpenSwiftUI)
for many cycles — the exact warned-against mistake. compare_bytes/compare/compare_heap_objects READ CLEAN
(8-aligned fast path + byte fallback correct). Bug, if any, is in the STRUCTURED path: the `Compare` functor
(Compare.cpp), `compare_indirect` (enum tag), `compare_partial` (struct walk) — wasm32 pointer-width / enum-
tag / offset assumptions. NEXT: read those + test compare in isolation for the reconciliation value types;
check whether oagcompare actually covers enum/heap-ref/struct cases (it passes, so likely does NOT cover the
failing case). This is a COMPUTE bug candidate (aligned with the goal), not OpenSwiftUI. Do NOT mask.

## Bug #13 (real, one-line, FOUND by reading compare per the rule) — `Compare` HeapRef pointer size hardcoded 8
`Compare.cpp:165` advanced the layout offset by a HARDCODED `8` after a HeapRef/Function field:
`size_t item_end = offset + 8;` (+ the diag `failed(...,8,...)` at :170). The layout BUILDER emits heap-ref/
function fields as `sizeof(void*)` (LayoutDescriptor.cpp:1239/1248) and the size-walker advances by
`sizeof(void*)` (LayoutDescriptor.cpp:689) — only the COMPARE reader hardcoded 8 (a 64-bit-ptr Apple-ism).
On wasm32 (`sizeof(void*)==4`) the compare over-advanced 4 bytes past every heap-ref field -> misaligned all
later fields -> wrong compare for any struct with a heap-ref-then-fields. FIX: `offset + sizeof(void *)` (+ :170).
Verify: full wasm suite green incl. oagcompare/oagvalues/oagrules (no regression).
HONEST: this did NOT fix the demo. Rebuilt+ran: CONTAINER-NEW still fires per-move (r=2,6,10,14,18), same crash.
So #13 is a real Compute bug (KEEP) but NOT #12's cause. (NOTE the `(uintptr_t)CompactNested base 0x1e3e6ab60`
in Compare.cpp:196 / LayoutDescriptor.cpp:711/868/1312 is SELF-CONSISTENT — base cancels mod 2^32 on encode/
decode + 32-bit ptr truncation — ugly but NOT a bug; leave it.)

## #12 UPDATE — compare primitive RULED OUT; root is the reconciliation/make path re-making the container
The strongest Compute hypothesis (compare_values, per [[feedback_change_detection_test_primitive]]) is
DISCONFIRMED: fixing #13 changed nothing — the DynamicContainer is STILL re-created every move. So the
recreation is NOT a value-compare reuse decision; it's view-tree RECONCILIATION actually re-MAKING the
ForEach/container subtree each move. Likely consumer (OpenSwiftUI) — but could involve a Compute make/
attribute-identity primitive. Demo structure: `GeometryReader { ZStack { ForEach(matrix.flatten(), id:\.tile.id)
{ ...position().transition(.blockGenerated) } } }`; `.transition` keeps a removed tile alive during its exit
anim, so a lingering OLD-container tile references the just-torn-down OLD container's LayoutChildGeometries.
NEXT: trace WHY the ForEach/DynamicContainer make re-runs every move — does GeometryReader re-make its
content ViewList on update (vs reconcile)? Read OpenSwiftUI GeometryReader make + ViewList reconcile path.
Probes still IN TREE (ROOT-TEARDOWN, CONTAINER-NEW, g_iag_render_tick) — REMOVE after #12. Do NOT mask.

DETERMINED (read DynamicLayoutView.makeDynamicView): the container + `LayoutChildGeometries`
(parentPosition: inputs.position) are created in `Layout.makeDynamicView` — a static MAKE function (runs at
view-tree construction, NOT per-update). CONTAINER-NEW firing 2x/move => makeDynamicView re-runs every move
=> OpenSwiftUI's view-list reconciliation is STRUCTURALLY RE-INSERTING the Layout container each move instead
of matching/reusing it. Combined with compare ruled out (#13 no-op on the symptom), #12 is an OPENSWIFTUI
RECONCILIATION defect (view-list structural matching re-makes the Layout/container subtree on a @State change),
NOT a Compute defect. Compute is behaving correctly; the consumer rebuilds a subtree it should reconcile, and
a lingering tile then reads the torn-down old container's geometry. FIX LAYER = OpenSwiftUI ViewList/DynamicView
reconciliation (why _makeView/_makeViewList re-runs for the Layout on update). Deep consumer work, separable
from the Compute-correctness goal. Resume: read OpenSwiftUI _makeViewList / DynamicView reconcile matching +
GeometryReader content handling; instrument which make (GeometryReader vs ZStack/Layout vs ForEach) re-runs.

# ═══ RESUMED 2026-06-28 (session 2) — #12 CONFIRMED OpenSwiftUI; targeting bug #14 = immortal-storage infidelity ═══

## #12 final confirmation (read-grounded, NOT a Compute bug — leaving it)
Re-verified by reading: reconciliation reuse-vs-remake is decided by ATTRIBUTE IDENTIFIER equality, not by
Compute's value-compare. `DynamicViewListItem.matchesIdentity` = `list == other.list && id == other.id`
(DynamicViewListItem.swift:28); `Attribute.==` = identifier equality (Attribute.swift:220). The Layout
container's `list` Attribute is RE-CREATED each move (`ViewList.makeAttribute` -> `ApplyModifiers` for a
modified dynamic list, ViewList.swift:1438; the eleev tiles carry `.position()`/`.transition`), so its
identifier changes -> matchesIdentity fails -> tiles re-made -> a lingering transition tile reads the
torn-down container's geometry. Compute correctly freed the dead subgraph's attributes. => #12 is an
OpenSwiftUI consumer defect; compare primitive is NOT implicated. Closed for the Compute goal.

## Bug #14 = IMMORTAL SUBGRAPH STORAGE is not faithful AttributeGraph
Off-Apple (`IAG_CF_STORAGE_SWIFT_MANAGED==0`, IAGBase.h) the `IAGSubgraphStorage` CF wrapper is NEVER freed
(the `CFRelease` at Subgraph.cpp clear_object + IAGSubgraph.cpp setCurrent are compiled out); on wasm the
foreign-ref retain/release hooks (`IAGSubgraphRetainRef/ReleaseRef`) are no-ops. Real AG frees storage via
ARC. => per-subgraph wrapper leak for process lifetime. GOAL: faithful free without UAF.

### Phase 0 (done): removed all 5 leftover #12 diagnostic probes (Subgraph.cpp ROOT-TEARDOWN; Graph.cpp
cstdio/cstring + g_iag_render_tick + CONTAINER-NEW). BASELINE GREEN both platforms: wasm 15/15, linux 15/15
(immortal storage in place). Added oag-baseline/run-suite.sh.

### Phase 1 (THE GATE) — measurement, shadow refcount, NO real free (instrument in IAGSubgraph.cpp,
gated IAG_DBG_SUBGRAPH_REFS). Mirrors create(+1), ARC retain/release hooks, current-ref retain/release, and
flags any release-to-<=0 while `storage->subgraph!=null` (= subgraph still ALIVE => a real free there = UAF).
- **Exp 1a (model the CURRENT shape):** ARC hooks DO fire (foreign-ref import works: e.g. oagteardown
  retains=67307/releases=74359). Globally BALANCED + leak-free (`created+retains==releases`,
  reached_zero==created, negative=0, leaked=0, max_live bounded). **BUT premature>0 EVERYWHERE**
  (oagchurn=3000=one per subgraph): refs transiently hit ZERO while the subgraph is ALIVE, because off-Apple
  the C++ graph owns subgraphs by RAW POINTER and ARC/current refs are only transient. => Driving real CF
  frees from the current shape = UAF. **The simple "free at ARC-zero" is DISPROVEN.** (This is exactly why
  immortal storage was adopted.)
- **Exp 1b (model a GRAPH-ALIVE self-ref):** register at refs=2 (one ARC-handle ref since create is
  RETURNS_RETAINED + one graph-alive self-ref); release the self-ref at `clear_object` (the subgraph's true
  death; reached on BOTH paths: explicit invalidate AND `Context::~Context`->`invalidate_and_delete_`,
  Context.cpp:38). RESULT: **premature=0, negative=0 EVERYWHERE.** Teardown-exercising tests free their
  subgraphs (oagteardown reached_zero=7051/7052, oagforeach 370/371), leaving only the root alive
  (leaked_alive=1). oagchurn leaked_alive=3000 is CORRECT — it deliberately keeps its graph alive
  (`_ = graph`, never invalidated). **GATE = GREEN: a graph-alive self-ref held create->clear_object makes
  refcount-driven freeing SAFE (no premature, no over-release) and faithful (freed at true death).**

### Phase 2 plan (faithful free, wasm first): (1) take a real graph-alive CFRetain at create; (2) real
CFRelease of it at clear_object; (3) make ARC hooks real (CFRetain/CFRelease); (4) flip the single gate
IAG_CF_STORAGE_SWIFT_MANAGED so the current-ref CFReleases compile back in. WATCH the finalize<->clear_object
reentrancy (finalize calls clear_object+invalidate_and_delete_; both must be idempotent — storage->subgraph
nulled + is_invalidating guard). Verify: suite green wasm+linux, heavy churn 0-crash/0-leak/BOUNDED memory,
demo >= prior frame. LINUX differs (no foreign-ref import -> ARC hooks don't fire; storage owned only by the
graph -> self-ref alone, freed at clear_object) — handle separately. Do NOT force a free if a path regresses.

### Phase 2+3 DONE (wasm) — IMMORTAL STORAGE REPLACED WITH FAITHFUL LIFECYCLE ✅
Implemented exactly the gate-proven model (all changes in Compute, wasm-gated; Apple unchanged; linux
unchanged/immortal):
- `IAGSubgraph.cpp` `IAGSubgraphRetainRef`/`ReleaseRef`: no-op -> REAL `CFRetain`/`CFRelease` (the wasm
  foreign-ref import's retain/release now actually drive the CF refcount).
- `IAGSubgraphCreate2`: take ONE extra `CFRetain(instance)` = the graph-alive self-ref (gated
  `#if defined(__wasi__) && IAG_CF_STORAGE_SWIFT_MANAGED`). Create's own +1 is RETURNS_RETAINED -> owned by
  the Swift handle; the extra +1 represents "alive in graph", released at death.
- `Subgraph::clear_object` (the true-death point; reached via explicit invalidate AND
  `Context::~Context`->`invalidate_and_delete_`): release the current-ref (existing gated CFRelease) FIRST,
  then the graph-alive self-ref CFRelease LAST (may free storage in-place; nothing touches `object` after).
- `IAGBase.h`: single gate `IAG_CF_STORAGE_SWIFT_MANAGED` = `__APPLE__ || __wasi__` (was `__APPLE__` only).
- Reentrancy SAFE (as predicted): the self-ref guarantees the refcount never hits 0 while the subgraph is
  alive, so `finalize` only ever fires AFTER `clear_object` nulled `storage->subgraph` -> finalize no-ops
  then frees. Confirmed: zero crashes across all detectors.
- Opt-in liveness counter (IAGSubgraph.cpp, env `IAG_STORAGE_LOG=1`, silent by default; `IAG_DBG_STORAGE_COUNT`).

VERIFICATION (all green):
- wasm oag-baseline 15/15 PASS, and storage is ACTUALLY FREED now (vs immortal=0-freed):
  `oagmemory created=6000 finalized=6000 live=0`, `oagteardown 7051/7052`, `oagweakref 1001/1002`,
  `oagforeach 370/371`; live==created only where the test deliberately keeps subgraphs (oagchurn 3000/0,
  singletons live=1). Bounded memory PROVEN.
- linux oag-baseline 15/15 PASS — UNCHANGED (all faithful-free paths gated off; linux stays immortal).
- OpenSwiftUI 2048 demo (heavy real-world churn): reaches render_frame #14 with the SAME pre-existing #12
  OpenSwiftUI crash (`invalid source attribute: 61093`, identical LayoutPositionQuery backtrace) — i.e. NO
  new/earlier UAF from faithful freeing; behavior-preserving up to the separate consumer bug.

### REMAINING (honest, bounded): LINUX still immortal — faithful freeing there needs the foreign-reference
import active on linux (so live Swift handles keep storage alive; without it, freeing at clear_object would
UAF a Swift opaque pointer that outlives invalidate — the original "oagteardown abort/page-recycle" symptom).
Extending the foreign-ref import to linux is entangled with the C++-interop migration (path A, documented
dead-end: library-evolution incompatibility + C-enum re-import). Linux is a TEST-HARNESS-ONLY platform (all
wandr consumers run on wasm), so this divergence is invisible to consumers; left as a precisely-characterized
gap, NOT a hidden hack. Follow-up if a linux consumer ever appears OR a newer SDK clears path A.

=> bug #14 (immortal Subgraph storage) is FIXED on the production target (wasm): Compute now frees subgraph
storage faithfully (AG semantics), bounded memory, zero UAF, full suite + heavy demo green.

# ═══ #12 deep dive (2026-06-28 session 2) — precise mechanism PINNED; NOT a Compute bug ═══
Goal of this dive: fix OpenSwiftUI #12 so the 2048 demo runs end-to-end. Used temporary probes (now ALL
removed; tree clean) in GeometryReader._makeView, _LayoutRoot._makeView, makeDynamicView, DynamicView
container/list re-make sites, and a Compute-side dump at the `add_input` precondition.

MEASURED FACTS (probe output, frame #14 crash):
- Crash dump: `reader=OpenSwiftUICore.LayoutPositionQuery readerSubIdx=20 readerValid=1 |
  input=61093 inputSubIdx=20 inputValid=1`. So the READER (LayoutPositionQuery) AND its source attribute
  61093 are in the SAME, VALID subgraph (idx 20). 61093 is a MUTABLE INDIRECT (parentPosition) that
  `resolve(EvaluateWeakReferences)` returns NIL for -> strict read (allow_nil=false) -> precondition.
- So it is NOT a dead-subgraph teardown race (subgraph is valid) and NOT the compare primitive.
- STRUCTURE: each `TileView` has its OWN GeometryReader (TileView.swift:50) — so LayoutPositionQuery
  exists per-tile AND for the board. The per-move GR-MAKEVIEW is an ENTERING tile (normal ForEach make),
  NOT the board re-making. (Earlier "board container re-made per move" reading was WRONG — it conflated
  board vs per-tile GeometryReaders.)
- REUSE/POOLING RULED OUT: DynamicLayoutViewAdaptor + DynamicViewListItem do NOT override
  `supportsReuse`(=false) / `maxUnusedItems`(=0) / `canBeReused`(=false). Tiles are never pooled/reused;
  removed tiles are invalidated, new tiles freshly made. So the crash is NOT stale-reuse wiring.

# ═══ #12 FIXES (2026-06-29) — 3 real Compute defects fixed via the detector method; systemic limit found ═══
Per task rules (detector + measurement, never mask), built a fast pure-Compute detector
`oag-baseline/oagoffset` (a cross-subgraph offset-projection read across a source-subgraph teardown +
page recycle) that reproduces the eleev-2048 frame-#14 crash in ~16s (vs 5-min demo rebuilds). Iterated:
  FIX #1 (Graph.cpp input_value_ref_slow): a strict @Attribute read THROUGH an indirect whose WEAK source
    EXPIRED (source subgraph torn down, page recycled -> zone-id weak-seed mismatch) now yields a zeroed
    default instead of aborting. Weak refs may legitimately expire; this matches AG (consumer keeps working,
    reconciliation repoints next pass). Detector case2 + LayoutPositionQuery (the original "invalid source
    attribute: 61093") FIXED. POD-safe.
  FIX #2/#3 (AttributeID.cpp resolve_slow UpdateDependencies): a mutable indirect's plain (non-weak)
    `_dependency` can dangle after its source teardown -> `update_attribute(dependency.get_node())` aborted
    (data::ptr operator-> on a null/dead node) and, when the page was recycled to a node already updating,
    formed a false cycle -> trapping `print_cycle` stub. Now the best-effort dependency pre-update is skipped
    unless the dependency is a LIVE node (non-null, page allocated, subgraph not invalidated, not already
    updating). ChildEnvironment (__assert_fail) + TextChildQuery (print_cycle) FIXED.
VERIFIED: oag-baseline 15/15 wasm + 15/15 linux (no regression); oagoffset detector PASSES both platforms;
demo advanced through 4 distinct frame-#14 manifestations (each fixed in turn).

SYSTEMIC LIMIT (manifestation #4/#5, NOT fixed — read-guards insufficient): after the above, the demo's
frame #14 next hits `propagate_dirty` walking a long-lived attribute's (DynamicContainerInfo asyncSignal)
`output_edges` that contain WILD entries (offset beyond the data table) -> a guard's own `raw_page_seed`
asserts. i.e. the page recycle during heavy per-move teardown CORRUPTS the data STRUCTURES themselves
(output-edge arrays), not just individual node values. Read-site guards cannot repair corrupted arrays.
ROOT (now clear): the jcmosc/Compute reimpl leaves dangling cross-subgraph EDGES on teardown (the
reverse-edge / non-mutable-indirect / dependency cleanup gaps), benign on 64-bit Apple (pages not recycled,
stale-but-intact) but corrupting on wasm (32-bit, page recycled+overwritten). The faithful fix is COMPLETE
teardown edge-cleaning (ensure remove_node removes a dying node from ALL holders' output_edges incl.
non-mutable-indirect/dependency paths) — a structural rework — OR a bounded page/subgraph quarantine
(memory tradeoff; the old immortal approach, regresses bug #14's faithful freeing). NOT a read-guard, NOT a
mask. The 3 fixes above are real, kept, verified. Reverted the futile propagate_dirty read-guard.
RESUME: complete teardown edge-cleaning so no dangling cross-subgraph output_edges survive a per-move tile
teardown (start: remove_removed_input/remove_removed_output non-mutable-indirect coverage + verify every
holder of a dying node clears its edge). Detector for that = extend oagoffset with a propagate_dirty case.

# ═══ #12 STRUCTURAL TEARDOWN INVESTIGATION (2026-06-29) — pervasive, multi-level; not point-fixable ═══
Pursued the faithful "structural teardown edge-cleaning" fix (user's explicit choice over quarantine).
Built detector oag-baseline/oagoffset case3 (long-lived source A; dependent B in another subgraph;
tear down B + recycle; mutate A -> propagate_dirty(A)): GENERIC teardown edge-removal is CORRECT (case3
PASSES wasm+linux). So the demo's leak is NOT a generic gap — it needs the demo's heavy per-move churn.
Drove the demo (8 instrumented rebuilds) to localize. Each read-side guard exposed the NEXT deeper layer
of the SAME corruption — confirming it is pervasive, not a single missing step:
  manifestation #4: propagate_dirty -> output_node->state() on a torn-down node.
  #5: same, output edge offset WILD (beyond data table) — guarded with plain bounds-check (raw_page_seed
      ITSELF asserts on a wild offset, so the earlier raw_page_seed guard crashed in itself).
  #6: output node page FREED (raw_page_seed==0) — guarded; then page LIVE but subgraph invalidated — guarded;
      then add_dirty_flags -> Subgraph::propagate_dirty_flags -> foreach_ancestor over a survivor subgraph's
      _parents containing a FREED parent Subgraph* (teardown left a dangling parent).
  next: a foreach_ancestor live-registry guard (Graph::contains_subgraph binary-search of the sorted
      _subgraphs) then crashed INSIDE contains_subgraph — i.e. the subgraph reaching it had a garbage
      `this->_graph`/_subgraphs: a RECYCLED page (raw_page_seed!=0, page->zone = a mid-churn/!valid zone).
ROOT (confirmed, deepest): the corruption is the teardown + wasm page-recycle LIFETIME MODEL, at EVERY
level — node values, node input/output edges, mutable-indirect dependencies, output_edge arrays, AND
Subgraph _parents/_graph. Compute frees subgraphs+pages on teardown (Subgraph IS-A data::zone; ~zone ->
clear() -> dealloc_page_locked, so a fully-deleted subgraph's pages read raw_page_seed==0; but RECYCLED
pages read !=0 and alias). The demo's per-move re-creation holds MANY bare (non-generation-tagged)
cross-subgraph refs into torn-down things. On 64-bit Apple all latent (pages not recycled -> stale-but-
intact). On wasm32 the page recycles+overwrites -> every traversal can hit garbage. Bare AttributeID /
Subgraph* refs carry NO generation, so a recycled offset/pointer is UNDETECTABLE at the read site (a guard
crashes one layer deeper). CONCLUSION: not fixable by localized read-guards. The faithful fixes are either
(a) make page-recycle SAFE — defer reclamation of a torn-down subgraph's pages until no ref remains
(refcount/quarantine; addresses ALL levels at once, but the user deferred this as a memory tradeoff), or
(b) generation-tag every cross-subgraph ref so recycle is detectable (large), or (c) a COMPLETE teardown
that provably removes EVERY cross-subgraph ref (node edges + dependencies + subgraph _parents) before any
free AND closes the recycle window — comprehensive, not the few-line edit "edge-cleaning" implies.
KEPT (real, verified, suite 15/15 wasm+linux): #1 input_value_ref_slow expired-weak read, #2/#3 resolve_slow
dependency guard, detector oagoffset (cases 1-3). All exploratory propagate_dirty/foreach_ancestor guards +
probes REVERTED (they moved the crash deeper, never reached a working game; not masking). RESUME: pick a
ROOT strategy (a/b/c above) — recommend (a) bounded page-recycle deferral, since it neutralizes all levels
at once and matches AG's effective non-recycling; build a churn detector that recycles aggressively to prove
it. Demo still aborts at frame #14 (was: same frame, now 6+ manifestations deeper before the recycle-window).

CORRECTION 4 (read path + final localization): the crash read is `Compute.Attribute.wrappedValue` ->
`IAGGraphGetValue` -> `get_value(seed=0)` -> `input_value_ref(subgraph_id=0)` -> strict
(`AssertNotNil`, `add_input allow_nil=false`). `subgraph_id` == the WEAK SEED: a plain `@Attribute`
read passes 0 (strict); only a `@WeakAttribute` read passes a non-zero seed (nil-tolerant). So
`LayoutPositionQuery.parentPosition` is a STRICT `@Attribute` (audited/faithful) -> making it tolerate
nil via subgraph_id/allow_nil would MASK the strict-read contract (rejected). The VALUE flows through a
NON-mutable OFFSET indirect (`childGeometry.origin()` = `[keyPath:\.origin]`) whose source is WEAK
(`add_indirect_attribute` non-mutable branch, `WeakAttributeID(attribute,...)`, Graph.cpp:516 — faithful
to upstream c0cc862). When the source `DynamicLayoutViewChildGeometry`'s subgraph is torn down (per-move),
the offset indirect's weak source expires; teardown can't reach the indirect (non-mutable -> no reverse
edge; reader's lazy edge not yet established; 0 cross-context-cascade hits) -> dangling -> strict read
preconditions. The strict read is AG-correct; the gap is the source LIFETIME (real AG keeps the offset
projection's source alive for the projection/reader, or repoints the surviving reader). FIX is therefore
one of: (a) offset(non-mutable) indirects hold a source-keeping-alive ref (vs weak) [broad memory-semantics
change], (b) repoint surviving readers' parentPosition to the new childGeometry on per-move reconciliation,
or (c) stop the per-move container/childGeometry re-make so surviving tiles' geometry source is stable.
NONE landed with confidence; crash is heap-sensitive/non-deterministic at the edges (readerValid flips
0/1) so crash-time probing has hit diminishing returns. NOT masked. Tree clean (all probes removed).
Honest state: #12 precisely localized, NOT fixed. Compute-correctness goal intact (bug #14 done; suite
15/15 wasm + 15/15 linux). RESUME: confirm (c) via an OpenSwiftUI-side semantic probe (which tiles
survive vs re-made per move + is their childGeometry re-made), then fix the reconciliation that re-makes a
surviving tile's geometry source.

CORRECTION 3 (ROOT, read-grounded — the mechanism, supersedes all guesses below):
`Graph::add_indirect_attribute` registers a reverse dependency on the source ONLY for MUTABLE indirects
(`add_input_dependencies`, Graph.cpp:508); the NON-mutable branch (511-522) does NOT — and this is
IDENTICAL in upstream jcmosc/Compute `c0cc862` (NOT a port regression). Non-mutable indirects (offset/
keyPath projections like `childGeometry.origin()`) rely on a READER establishing an edge: `resolve_slow`
FOLLOWS THROUGH non-mutable indirects (the `is_mutable()` block at AttributeID.cpp:60 is skipped), so a
reader's `add_input_dependencies` registers the reverse edge on the real source. That edge is what lets
subgraph teardown reach + repoint/invalidate the dependent.
THE CRASH = a lazy/ordering hole in that scheme: the tile's `LayoutPositionQuery` had NOT yet read
`parentPosition` (lazy edge) when the `DynamicLayoutViewChildGeometry`'s subgraph was torn down and its
page reused by another subgraph (cross-render). With no reader edge AND no reverse edge for the
non-mutable projection, teardown never reaches the projection -> it dangles -> the first read resolves the
expired weak source -> `add_input` strict precondition (AG-correct) -> abort. renderOnce's
`withoutSubgraphInvalidation` defers teardown WITHIN a render but the staleness is ACROSS renders, so it
can't catch this.
=> This is a deep cross-subgraph teardown/update-ORDERING divergence (a surviving reader first-reads a
source whose subgraph was already torn down + page-recycled). It is the hardest #12 layer. LOCALIZED but
NOT a confident one-change fix: a faithful fix means matching real AG's teardown/eval ordering or
dependent-invalidation so a referenced source is not recycled out from under a not-yet-read reader (real
Apple AG handles it; the jcmosc reimpl + this wasm port have the gap). Refused (per rules) to ship a mask
(nil-tolerant LayoutPositionQuery) or a speculative reverse-edge/ordering patch without proof. All DV12
probes removed; tree clean. Resume from: instrument update/eval ORDER of LayoutPositionQuery's first
parentPosition read vs the childGeometry subgraph teardown across renders N/N+1.

CORRECTION 2 (deepest probe — supersedes BOTH guesses below): `curOccupant` of the expired source slot =
`DynamicLayoutViewChildGeometry`, and `WeakAttributeID::expired()` checks the **page's zone-id** (a page
freed and reused by a DIFFERENT subgraph). So 61093 = a tile's `parentPosition = childGeometry.origin()`
weak projection of a `DynamicLayoutViewChildGeometry` (the board container's per-tile geometry); that
childGeometry's **SUBGRAPH was torn down** (its pages returned to the table and reused by a new subgraph),
so the projection expires and the tile's `LayoutPositionQuery` strict-reads it -> precondition. RULED OUT
(all measured): reuse/pooling (supportsReuse=false, maxUnusedItems=0), dead-target-mutable-repoint
(it's a NON-mutable weak projection), within-subgraph node recycle (it's page-zone-id reuse across
subgraphs), AND view-transitions (WandrApp.swift sets supportsViewTransitions=false -> transitions are
NOT made; the `.transition` traits render directly). So #12 = a SURVIVING tile's `parentPosition` weak-refs
a `DynamicLayoutViewChildGeometry` whose subgraph was torn down/reused across a render — a deep OpenSwiftUI
reconciliation **lifetime** bug (Compute's page-zone-id weak-expiry is the correct UAF guard; the strict
read precondition is AG-correct). renderOnce's `withoutSubgraphInvalidation` defers teardown WITHIN a render
but the stale ref survives ACROSS renders. IMPASSE: a confident correct fix needs OpenSwiftUI reconciliation
surgery (keep a surviving tile's childGeometry lifetime == the tile, or repoint/invalidate parentPosition
when the container re-makes per-tile geometry) — not landed; refused to ship a nil-tolerant LayoutPositionQuery
(masking). Probe `iag_dv12_*` still in Graph.cpp add_input/add_attribute/remove_node (wasm-gated) — remove
when resolved. (Older guesses below retained for history.)

CORRECTION (sharper probe at the crash, dumping the indirect's source — supersedes the "mutable repoint"
guess above): the crashing `input=61093` is a **NON-MUTABLE WEAK indirect** (mutable=0) whose source
`61032` is **EXPIRED** (`srcExpired=1`, srcIsNil=0) — i.e. the source node was destroyed and its data-zone
slot RECYCLED (weak-seed mismatch) — and BOTH 61093 and 61032 live in the SAME, still-VALID subgraph 20.
So it is NOT a mutable-indirect-repointed-to-nil and NOT a cross-subgraph dead target. It is a WEAK
reference to a RECYCLED node within one live subgraph. `LayoutPositionQuery.value` reads `localPosition`
first, and `localPosition = rootGeometry.origin()` (GeometryReader.swift:52) — so 61093 is almost certainly
the tile's own `rootGeometry.origin()` weak-projection, and 61032 its `RootGeometry`, which got recycled
mid-render (the per-render GeometryReader content re-make / settling burst recycles the prior RootGeometry
while a sibling LayoutPositionQuery still weak-reads it). NEXT to fully pin: instrument node destruction to
see WHO recycles 61032 while 61093/the reader survive (open-ended; needs node-destroy tracing). FIX layer
still TBD between Compute (weak-ref-to-recycled-node should invalidate readers / not recycle while weakly
held) and OpenSwiftUI (GeometryReader make ordering / re-make recycling RootGeometry under a live reader).
Compute strict-read-of-expired-weak -> precondition is AG-correct; the bug is the premature recycle +
surviving reader. Do NOT mask (no nil-tolerant LayoutPositionQuery). Probe still in Graph.cpp add_input
(`iag_dv12_dump`, wasm-gated) — remove after the recycle cause is found.

FIX DIRECTION (for whoever resumes #12; deep consumer work): ensure a removed/transitioning tile's
geometry-dependent rules are not evaluated against a dead parent — either (a) invalidate the indirect's
readers when it is repointed-to-nil so the surviving reader is torn down with its dead source, or (b)
repoint the parentPosition indirect to a live source (or freeze the last value) when the source dies during
a transition. NOT attempted as a one-line patch (would risk masking / cycling). Do NOT change
LayoutPositionQuery to a nil-tolerant @OptionalAttribute read — that diverges from audited Apple behavior
and masks the lifecycle bug. Demo still reaches frame #14 (13 frames render correctly); Compute work stands.
(Stale artifact note: repros/swift-canvas-spike/openswiftui-demo.component.wasm on disk is a probe build;
a clean `build-openswiftui-demo.sh` regenerates it.)

================================================================================
BUG #14 — render_frame #14 crash ("unknown handle index 1668183366" / canvas_save)
  ROOT CAUSE FOUND + FIXED (2026-06-29). This is the crash that previously read as
  "#12 frame #14" — it is a DISTINCT, genuine Compute defect, NOT the GeometryReader
  weak-recycle lifecycle issue described in the #12 notes above.

ROOT CAUSE (one uninitialized field):
  `IAG::IndirectNode`'s PUBLIC (immutable) constructor
  (Attribute/AttributeData/Node/IndirectNode.h:37) did NOT initialize the `_mutable:1`
  bitfield (no default member initializer; only the protected ctor used by
  MutableIndirectNode set it). So an immutable IndirectNode allocated in a RECYCLED
  data-zone page kept a STALE `_mutable == 1`. Then `is_mutable()` lies → the node is
  `unsafe_cast<MutableIndirectNode>` (Graph.cpp:783, add_input_dependencies →
  add_output_edge<MutableIndirectNode>) and `output_edges()` reads PAST the smaller
  IndirectNode allocation into adjacent memory → a garbage `data::vector<OutputEdge>`
  (_data == null, but stale `_metadata` with capacity_exponent e.g. 20 / size 4..7).
  `reserve()` sees `size+1 <= capacity()` and NO-OPS (never allocates), so `push_back`
  writes `&data()[size] == nullptr + size*4` = LOW memory (the wasm globals/data
  segment), corrupting an adjacent global (observed: the demo's `sink.cg` CGContext
  pointer -> 63064 -> cg.canvas reads "Func"=1668183366 -> canvas_save traps).
  In release the `assert(is_mutable())` is compiled out; on 64-bit Apple the memory
  is fresh/zeroed so _mutable reads 0 -> latent. wasm32 recycles pages -> fatal.

FIX (faithful, one line):
  Attribute/AttributeData/Node/IndirectNode.h — add `_mutable(false)` to the immutable
  IndirectNode ctor init list. (Initialize the field that was left uninitialized.)

METHOD (how it was pinned — reusable):
  - wandr-host gained an ENV-GATED `Config::debug_info(true)` (WANDR_DEBUG_INFO=1),
    desktop-only, so wasmtime registers JIT code with gdb's JIT interface -> gdb can
    SYMBOLIZE guest (wasm) frames. (runtime/wandr-host/src/lib.rs make_config; off by
    default; zero device/AOT impact.)
  - gdb hardware WATCHPOINT on the corrupted native address (wasm linear-base, pinned
    via a unique guest SENTINEL searched in /proc maps, + the field offset) caught the
    corrupting STORE with a full symbolized backtrace (the trap itself is not a native
    fault — wasmtime catches it — so a watchpoint is the only way).
  - A temporary `data::vector::push_back` measurement (log when `_data` is a low/garbage
    pointer) revealed the null-_data + stale-_metadata state, which led to the
    is_mutable()/OOB-read root. All scaffolding removed after the fix.
  - cgwatch*.gdb scripts live in swift/OpenSwiftUIProject/ (reusable for future
    wasm-specific corruption hunts).

RESULT: demo no longer crashes at frame 15. It RENDERS the real 2048 board
  (DRAWCOUNT frame=200 shapes=22 texts=21) and runs ~25x further, to frame ~383.

NEW FRONTIER (separate, pre-existing bug, previously masked by #14):
  render_frame #383 — `swift_deallocClassInstance` fatalError via
  `util::cf_ptr<IAGSubgraphStorage*>::~cf_ptr -> CFRelease` inside
  `IAG::Subgraph::update(unsigned char)` (the stack<cf_ptr<IAGSubgraphStorage*>>
  destructor) <- GraphHost.finishTransactionUpdate. This is the immortal-storage /
  faithful-subgraph-refcounting area (an over-release / double-free of subgraph
  storage). Distinct from #14; next to investigate.

================================================================================
BUG #383 — ROOT PROVEN, fix pending (subgraph-storage over-release in Subgraph::update)
  After #14, the demo renders the 2048 board and runs to ~frame 383, where it aborts:
  "Object 0x... deallocated with non-zero retain count 77921 ... deinit created a strong
  reference to self" — i.e. an IAGSubgraphStorage is DOUBLE-FINALIZED.

MEASURED (env-gated traces, since removed):
  - The crashing storage is FINALIZED TWICE: first with storage->subgraph still SET + is_valid()==1
    (a PREMATURE free of a live subgraph), then re-entrantly (subgraph now null).
  - Finalize #1 runs clear_object(), whose self-ref `CFRelease(object)` fires ON THE STORAGE BEING
    FINALIZED -> re-enters dealloc -> finalize #2 -> the "non-zero retain count" abort. (Proximate.)
  - Underlying: the storage hit refcount 0 while the subgraph was alive. Per-storage refcount tally:
    CREATE=1, RETAIN-HOOK=11/RELEASE-HOOK=11 (balanced handle ARC), SETCUR-RET=1/SETCUR-REL=1
    (balanced current-ref), but **CFPTR-RET=4 vs CFPTR-REL=6** — the `cf_ptr` stack in
    Subgraph::update releases the storage TWO MORE TIMES than it retains (+1 over-release per update
    cycle; the extra REL is a leftover in the std::stack destructor at update end).

CONTROLLING TEST (decisive): the update stack is
  `std::stack<util::cf_ptr<IAGSubgraphRef>, IAG::vector<util::cf_ptr<IAGSubgraphRef>, 32, uint64_t>>`
  (Subgraph.cpp:~669). Bumping the inline capacity 32 -> 65536 so the IAG::vector NEVER reallocs
  ELIMINATES the over-release entirely: 0 premature finalizes, no abort, demo runs to AUTOPLAY
  1500/5000 (frame ~4500). => ROOT = IAG::vector's memcpy/realloc relocation interacting with
  cf_ptr's non-trivial (CFRelease) destructor leaves a duplicate cf_ptr that gets double-released.

STATUS: root proven, NOT yet fixed. The generic IAG::vector push/pop/realloc all *read* as correct
  for a trivially-relocatable cf_ptr, so the exact memcpy bug is subtle; a blind change risks the
  15/15 oag-baseline. NEXT: isolate it with a standalone IAG::vector<refcount-probe, small_cap> unit
  test (push past inline cap, pop, assert ctor==dtor balance) per the "test the primitive" rule, then
  fix realloc_vector (or make cf_ptr relocation provably sound). The 32->65536 capacity bump is a
  documented stopgap, NOT the fix (still latent for >cap and for other cf_ptr vectors).

  -------- #383 FIXED (2026-06-30) --------
  EXACT BUG (isolated + ASAN-proven in swift/OpenSwiftUIProject/vector-cfptr-test.cpp): IAG::vector's
  inline buffer is a `T _inline_buffer[_inline_capacity]` MEMBER ARRAY. On the first inline->heap grow,
  realloc_vector memcpy's the inline elements to the heap buffer but LEFT the inline source intact. The
  live elements now live on the heap (destructed by ~vector's explicit loop via data()==heap), but when
  the vector object dies the COMPILER also auto-destructs the `_inline_buffer[]` member -> runs ~T() a
  SECOND time on those already-relocated slots. For util::cf_ptr that second ~cf_ptr is a double
  CFRelease -> the subgraph storage's refcount drops below its self-ref floor -> premature finalize
  while the subgraph is still alive -> clear_object()'s self-CFRelease re-enters dealloc -> double
  finalize -> "deallocated with non-zero retain count" abort. (The isolated test reproduces exactly:
  WITHOUT fix -> 4 storages hit refcount -1 at stack-dtor; WITH fix -> balanced at 5001 nodes.)

  FIX (Vector/Vector.h realloc_vector, inline->heap branch): after the memcpy, zero the inline source —
  `memset(_inline_buffer, 0, (*size) * element_size_bytes);` — so the auto-destruction of the moved-from
  slots is a no-op (zero == the moved-from state these trivially-relocatable elements assume; safe for
  all current element types: cf_ptr, raw pointers, data::ptr, OutputEdge).

  RESULT: demo no longer aborts at #383. Runs continuously with NO crash — rendered the live 2048 board
  to DRAWCOUNT frame=6600 (shapes=54 texts=52), AUTOPLAY 1000+/5000, 0 errors (cut off by timeout, not a
  crash). Both #14 and #383 fixed; the eleev 2048 demo plays continuously on wasm.

================================================================================
★★★ MILESTONE (2026-06-30): eleev/swiftui-2048 PLAYS CONTINUOUSLY ON WASM ★★★
  The OpenSwiftUI + Compute (AttributeGraph reimpl) stack now runs the real 2048 game on
  wasm32-wasip1 with ZERO crashes. Two genuine, independent AttributeGraph defects were found
  and fixed this session (both measurement/ASAN-proven, both faithful — no masks):

    #14  IndirectNode immutable ctor left `_mutable` uninitialized -> is_mutable() lies on a
         recycled page -> immutable node cast to MutableIndirectNode -> output_edges() OOB write.
         FIX: `_mutable(false)` (one field).  Was: crash at render_frame #14.

    #383 IAG::vector inline->heap realloc memcpy'd elements to the heap but left the
         `T _inline_buffer[]` MEMBER ARRAY intact -> compiler auto-destructs it -> double ~cf_ptr
         -> double CFRelease -> subgraph-storage double-finalize -> abort.
         FIX: `memset(_inline_buffer, 0, ...)` after relocation (one line). Isolated ASAN repro =
         swift/OpenSwiftUIProject/vector-cfptr-test.cpp.  Was: crash at render_frame #383.

  EVIDENCE: demo renders the live board to frame 6600+ (54 shapes / 52 texts), AUTOPLAY 1000+/5000,
  0 errors (timeout-bounded, not a fault). Regression: wasm oag-baseline 15/15 PASS, 0 FAIL.
  Method that cracked both: an env-gated wasmtime debug_info in wandr-host (desktop-only,
  WANDR_DEBUG_INFO=1) for gdb JIT symbols + hardware watchpoints, and isolating the primitive.
================================================================================

================================================================================
★★★ GESTURE / POINTER INPUT — Phase A (2026-06-30) — IN PROGRESS ★★★
  GOAL: feed host pointer events into OpenSwiftUI's gesture pipeline so .onTapGesture /
  DragGesture fire (real framework hit-testing), replacing the demo's hand-rolled draw-rect
  input routing. The whole event->responder->gesture-graph subsystem was ~20
  `_openSwiftUIUnimplementedFailure()` stubs; it is now implemented and COMPILES, and the
  responder tree assembles, renders, and survives the wasm walls. A tap traps; we peel the
  trap bugs one at a time. TWO found so far:

  --- BUG G1: StatefulRule.attribute read inside Subgraph.apply — FIXED (OpenSwiftUI 2fa39812) ---
    AnyGestureInfo.makeItem (AnyGesture.swift) read its own rule's `attribute`
    (= Compute StatefulRule.attribute = Attribute(identifier: AnyAttribute.current!)) INSIDE
    childGraph.apply { }. Subgraph.apply calls Graph.clearUpdate() which TAGS the current update
    stack (IAGGraphClearUpdate, IAGGraph.cpp:768) so IAGGraphGetCurrentAttribute() returns nil
    BY DESIGN (constructing a subgraph is not evaluating an attribute). The force-unwrap on nil
    traps (SIGILL / `unreachable`).
    FIX (faithful, no Compute change): capture `let containerAttribute = attribute` BEFORE
    entering childGraph.apply (valid there — makeItem runs from updateValue, current == self),
    use the captured value inside. VERIFIED: the dispatch crash moves PAST this point.
    (Detour, reverted: a speculative "cross-graph call_update tagged-stack" hypothesis + a
    Graph.cpp guard — WRONG path; the InTransaction update skips call_update entirely. Lesson:
    the symbolized backtrace settled it instantly; go there BEFORE theorizing.)

  --- BUG G2: PointerOffset.of returns a GARBAGE offset for non-trivial fields — NEXT TARGET ---
    Map2Gesture._makeGesture (Map2Gesture.swift:130/134) projects its struct fields into indirect
    attributes: `modifier[offset: { .of(&$0.content) }]` and `{ .of(&$0.body) }`. Compute rejects
    it (Graph::add_indirect_attribute, Graph.cpp:487):
        precondition failure: invalid size for indirect attribute:
        attr_size=100 offset=48279392 size=8 base_off=16
    offset=48279392 (~48 MB) is GARBAGE — a stack temporary address — and size=8 is the `body`
    CLOSURE. ROOT CAUSE: PointerOffset computes offsetof(field) via the trick
    `&invalidScenePointer().pointee.field - invalidScenePointer()` (Compute
    Attribute/PointerOffset.swift), where invalidScenePointer() = a fake pointer at a fixed low
    address (MemoryLayout<Base>.stride). The trick REQUIRES `&...pointee` to stay in-place. On
    wasm, because Map2Gesture is NON-TRIVIAL (holds a closure), `&invalidScenePointer().pointee`
    materializes a temporary COPY (~48 MB) instead of projecting in place, so the relative
    subtraction yields garbage. (.content is deferred; .body — forced via `.value` — trips first,
    but both are affected.) NOT gesture-specific or agent-introduced: a Swift/wasm codegen issue
    in PointerOffset projection for ANY non-trivial type; gestures are just the first to project a
    CLOSURE field this way. Likely broader blast radius. FIX = a materialization-free offset
    computation (open). FIRST STEP next session: confirm whether PointerOffset.of works for
    closure/non-trivial fields on LINUX (materializes there too, or wasm-only codegen?); read the
    Swift `&ptr.pointee` (_modify accessor) lowering on wasm; then fix PointerOffset.of/.offset to
    address fields without materializing the fake base.

  --- BUG G2: DIAGNOSED (2026-06-30) — root cause MEASURED, not wasm-specific, blast radius = 1 site ---
    Repro = repros/openswiftui-wasm/pointeroffset-probe/ (self-contained: vendors a VERBATIM copy of
    Compute/.../PointerOffset.swift; pure-stdlib so it dual-builds native + wasm with zero deps).
    It runs PointerOffset.offset { .of(&$0.field) } over 4 field shapes vs MemoryLayout.offset(of:)
    (ground truth) + a materialization detector (inout base == invalidScenePointer?).
    MEASURED MATRIX (identical conclusion native↔wasm):
      | field shape              | native -Onone | native -O | wasm32         |
      | trivial (Int)            | OK            | OK        | OK             |
      | struct-holding-closure   | OK            | OK        | OK             |
      | class reference          | OK            | OK        | OK             |
      | BARE function (A,B)->C    | SEGFAULT      | SEGFAULT  | GARBAGE offset |
    ROOT CAUSE (definitive): NOT a wasm codegen issue and NOT optimization-dependent. Taking the
    address of a BARE FUNCTION-TYPED stored property reabstracts it: `&$0.body` (and
    `MemoryLayout.offset(of: \.body)`) does NOT yield the in-place field address but a
    reabstraction-thunk TEMPORARY. Proof: with a REAL zero-initialized Base buffer, `&buf.body`
    returns an address ~168 B BELOW the buffer (a stack temp), not buf+8; `offset(of:\.body)`=nil
    (while \.head=0 and struct/class fields are correct). So PointerOffset's `&...pointee.field`
    trick CANNOT compute a bare-function field's offset on ANY platform — wasm just turns the
    segfault into a readable-garbage offset (low linear-memory addr). The reabstraction temp is
    formed at the CALL SITE `{ .of(&$0.body) }`, BEFORE `.of` runs → the Compute primitive cannot
    fix it (plan Approach A dead); `offset(of:)`=nil → the `[keyPath:]` subscript can't either
    (Approach B dead). OAG #70 ("PointerOffset.of crash", PR #71) fixed only the BY-VALUE-copy
    instance (`withUnsafePointer(to: member)` → `to: &member`); our live Compute already has that —
    necessary but insufficient for the function-field case (still open upstream).
    BLAST RADIUS = exactly ONE site: `Map2Gesture.body` (Map2Gesture.swift:121, projected :134) is
    the ONLY bare-function field projected via `.of` in the whole tree. MapGesture/VariadicView have
    function-typed `body` fields but project STRUCTS (`_body.modifier`, `.root`), never the closure;
    CallbacksGesture already wraps its body in a `_Body` STRUCT (projects fine) — the fix pattern the
    codebase itself already uses.
    FIX (Approach Z, faithful to that `_Body` convention): wrap `Map2Gesture.body`'s bare closure in
    a single-field struct so the `.of` projection targets an addressable struct (proven correct).
    Map2Phase then holds `@Attribute var body: <wrapper>` and invokes `body.call(phase1, phase2)`.
    APPLIED + VERIFIED (2026-06-30): Map2Gesture.swift — added `fileprivate struct Map2GestureBody
    <InputValue, ContentValue, OutputValue> { var call: (GesturePhase<InputValue>,
    GesturePhase<ContentValue>) -> GesturePhase<OutputValue> }`; `body` field + Map2Phase `@Attribute
    var body` retyped to it; construction site wraps `Map2GestureBody(call: body)`; Map2Phase reads
    `body.call(phase1, phase2)`. NOTE: `modifier[offset:{...}].value` keeps the `.value` —
    `modifier[offset:]` returns `_GraphValue<Member>` and `.value` is the underlying `Attribute<Member>`
    (structurally required, NOT a probe artifact). OpenSwiftUICore+OpenSwiftUI compile clean for wasm.
    SYNTH-TAP VERIFY (WANDR_DEBUG_SYNTH_TAP=1 WANDR_DEBUG_INFO=1, named component): the
    `invalid size for indirect attribute` precondition (Graph.cpp:487) is GONE (grep count 0); the
    Map2Gesture body indirect-attribute now builds and the tap dispatch advances PAST
    Map2Gesture._makeGesture into the responder/gesture graph. Regression guard kept =
    repros/openswiftui-wasm/pointeroffset-probe (dual native+wasm; `swift run <case>` /
    `wasmtime run ... <case>`; cases: trivial struct func class wrapped offsetof func_* ).

  --- BUG G3: LayoutGesture._makeGesture is an unimplemented stub — NEXT TARGET ---
    With G2 fixed, the synth-tap symbolized backtrace now traps at
    `OpenSwiftUICore/LayoutGesture.swift:24: Fatal error: Unimplemented yet` —
    `_openSwiftUIUnimplementedFailure()` in `LayoutGesture._makeGesture(gesture:inputs:)`
    (via DefaultLayoutGesture._makeGesture ← Gesture.makeDebuggableGesture ←
    DefaultLayoutViewResponder.makeGesture ← GestureResponder.makeSubviews ← SubviewsPhase.updateValue).
    This is a genuine STUB to IMPLEMENT (a real _makeGesture body), NOT a codegen/offset bug — a
    different class from G1/G2. Next: implement LayoutGesture._makeGesture faithfully (read the
    DefaultLayoutGesture/LayoutGestureChildProxy path + how DefaultLayoutViewResponder expects the
    gesture outputs) rather than patch-to-green.
    FIXED + VERIFIED (2026-06-30): LayoutGesture._makeGesture now returns `inputs.makeDefaultOutputs()`
    (LayoutGesture.swift). Reasoning (read end-to-end, not guessed): the ACTUAL .onTapGesture is built
    by GestureResponder.makeGesture (the override, GestureViewModifier.swift:374) via the modifier's
    content gesture — NOT DefaultLayoutGesture. DefaultLayoutGesture._makeGesture (the stub) is reached
    only on the BASE responder path (GestureResponder.makeSubviews → DefaultLayoutViewResponder.makeGesture
    @63 → DefaultLayoutGesture._makeGesture) for plain non-gesture layout subviews, where the gesture has
    Value==(), an empty updateEventBindings, and no phase behavior. makeDefaultOutputs() (GestureInputs.swift:113)
    is the API's own factory for exactly that (a DefaultRule phase + indirect preference outputs); the caller
    then overrideDefaultValues() with it. So it is the faithful minimal output, not a mask. The richer
    LayoutGestureChildProxy/updateEventBindings machinery stays WIP upstream. Synth-tap: LayoutGesture.swift:24
    trap GONE (grep 0); dispatch advances PAST graph BUILD into EVENT DISPATCH — GTRACE shows
    VG.sendEvents → GestureGraph.sendEvents → runTransaction → (next trap). Tap event now flows through the
    gesture pipeline.

  --- BUG G4: DurationGesture.updateValue force-unwraps a nil `elapsed` during dispatch — NEXT TARGET ---
    With G2+G3 fixed the synth-tap event reaches GestureGraph.sendEvents; during the transaction update,
    `DurationGesture.updateValue` (DurationGesture.swift) traps at `let elapsed = elapsed!` (line ~112 .active /
    ~120 .ended): "Unexpectedly found nil while unwrapping an Optional". elapsed is derived from `start`/`time`
    (elapsed = time - start, else .zero/nil depending on childPhase) — nil here points at the gesture clock/
    `time`/`start` wiring (consistent with the threading-shim note that withDelay timers don't fire yet on wasm).
    Trips at a LATE frame (~12739), i.e. after the down dispatches, when the duration gesture's active/ended
    timing is evaluated. Class = gesture timing/clock, distinct from G1-G3. NEXT (read source first): trace
    DurationGesture's `time`/`start` attributes + how the gesture clock is fed on wasm before patching.
    FIXED + VERIFIED (2026-06-30): root cause MEASURED via flushed #if os(WASI) traces in
    DurationGesture + EventListener (fflush(nil) — fatalError/abort does NOT flush guest stdout, so
    traces near a crash are lost unless flushed; earlier GTRACE survived only by printing thousands of
    frames before the trap). Trigger = a REAL window CLICK (user-confirmed: "trap only when I click"),
    NOT the synth-tap (down-only = .began → .possible, never .ended). A quick click is down+up with NO
    drag, so the pointer pipeline emits began → ended with NO intervening .active frame. EventListener
    mirrors event phase → emits .ended directly (skipping .active). DurationPhase.updateValue then hits
    `.ended` with start==nil (start is only set on .active, line 90) → `let elapsed = elapsed!` traps.
    DurationGesture.swift is "Status: Complete" (faithful) but its `elapsed!` assumes an .active set
    start first; an active-less click violates that. FIX (faithful, same decision point as line 89):
    also start timing when `childPhase.isEnded` (not only .isActive/trackFromEventStart) — an active-less
    terminal gesture has elapsed == 0, so .ended computes `.ended(0)` and (for a tap, minimumDuration 0)
    SUCCEEDS. One-line change (+ comment) in DurationGesture.swift; EventListener trace reverted.
    VERIFIED: clicking the desktop window now logs `WANDR-DEMO: TAP-FIRED count=1` (the .onTapGesture
    FIRES) with NO trap and no log flood; GTRACE shows the full clean dispatch
    GG.sendEvents → runTransaction → subgraph.update → TAP-FIRED, rendering continues. This is the
    Phase-A GOAL: real OpenSwiftUI gesture hit-testing firing a tap from a host pointer event.
    (Note: the down-only synth-tap can't reproduce the .ended path — a real click / down+up does.
    Deeper question for later: whether the input pipeline SHOULD emit an .active for a held press;
    the fix is correct regardless since an active-less gesture genuinely has zero duration.)

  --- DRAGGESTURE PART A: the .active path (pointer moves → gesture pipeline) — DONE + VERIFIED ---
    GOAL (split): Part A = make .active phases flow into OpenSwiftUI gestures (prereq for DragGesture);
    Part B = implement `struct DragGesture` (NOT present in this port — only TapGesture/SpatialTapGesture/
    DistanceGesture/DefaultLayoutGesture/SubviewsGesture exist; SpatialEvent + DistanceGesture primitives
    are there to build on). FINDINGS (read end-to-end): the host→guest .active mapping already exists
    (WandrApp.swift:97-101 — phase 1 → EventPhase.active) and the host dispatches KIND_MOVE on every
    CursorMoved (lib.rs:969, regardless of button) / TouchPhase::Moved (lib.rs:943). The ONLY gap: the
    demo's onPointer (repros/swift-canvas-spike/.../main.swift) handled only DOWN/UP and dropped
    KIND_MOVE in `default: break`, so wandrSendPointer(phase:1) was never called → the gesture pipeline
    only ever saw began → ended (this is also why G4's DurationGesture hit .ended with no .active).
    FIX (Part A, demo only): track a `pointerPressing` flag (set on DOWN, cleared on UP) and forward
    KIND_MOVE → wandrSendPointer(phase:1) WHILE pressing (a real began→active…→ended drag, not
    hover-flooding). Purely additive — the hand-rolled swipe still uses the down→up delta.
    VERIFIED on desktop (temp flushed [ELTRACE-A] in EventListener, since reverted): a press-drag-release
    produced phase=began ×3 then phase=ACTIVE ×33, received by BOTH EventListener<TappableEvent> AND
    EventListener<SpatialEvent> (the drag event type), no trap, and correctly NO TAP-FIRED (a moving drag
    is not a tap). The .active path now flows end-to-end into the gesture pipeline.
    NEXT = Part B: implement `struct DragGesture` faithfully on EventListener<SpatialEvent> + DistanceGesture
    (min-distance), then drive a real drag in the demo. (Note: the hand-rolled draw-rect input routing in
    onPointer STILL drives actual 2048 gameplay; the gesture pipeline remains a parallel probe until Part B
    wires a real gesture to behavior.)

  --- DRAGGESTURE PART B: implement `struct DragGesture` — DONE + VERIFIED ---
    DragGesture did NOT exist in the port. Implemented it (OpenSwiftUICore/Event/Gesture/DragGesture.swift,
    public) mirroring DistanceGesture: a GestureStateProtocol StateType (sticky startLocation + maxDistance
    for the minimumDistance gate) + `body = StateType.gesture(content: EventListener<SpatialEvent>()) { state,
    phase in … }` mapping SpatialEvent phases → GesturePhase<DragGesture.Value>. Value = SwiftUI-shaped
    {time(Date from SpatialEvent.timestamp.seconds), location, startLocation, velocity(.zero — not yet
    estimated), translation(computed), predictedEnd*(computed = current, no projection yet)}. Built on the
    pieces already present: EventListener<SpatialEvent>, DistanceGesture's distance(_,_), GestureStateProtocol/
    StateContainerGesture, CoordinateSpace, .onChanged/.onEnded (CallbacksGesture), .gesture<T>(_:) (GestureMask).
    Demo probe: `.gesture(DragGesture(minimumDistance: 0).onChanged{…}.onEnded{…})` on the hint Text logs
    DRAG-CHANGED/DRAG-ENDED with translation.
    VERIFIED deterministically via a new host harness WANDR_DEBUG_SYNTH_DRAG=1 (lib.rs, sibling of synth-tap):
    injects down@frame2, 8 moves@frames3-10 (+15px/step), up@frame11 — no dependence on flaky desktop window
    focus (two interactive rounds saw 0 events; synth-drag is reliable). Result: DRAG-CHANGED dx=15…120 (8×,
    one per move, correct cumulative translation from the start) then DRAG-ENDED dx=135, no trap. onChanged
    streams during the drag + onEnded on release = the .active path drives a real DragGesture end-to-end.
    NEXT (optional): velocity/predicted-end estimation; coordinate-space transform; wire the demo's 2048 swipe
    onto DragGesture (retire the hand-rolled draw-rect swipe). The hand-rolled routing still drives gameplay.

  --- DRAGGESTURE PART B / PATH 2: geometric (per-view) hit-testing — pieces 1+2+3 DONE + VERIFIED ---
    GOAL: a gesture fires ONLY when its own view is hit (per-view location hit-testing), replacing the
    structural "first gesture regardless of location" fallback. SCOPING (2 Explore agents): the WHOLE
    SwiftUI responder/hit-test scaffolding is already ported + trap-free (ViewResponder.hitTest recursion,
    containsGlobalPoints/BitVector64 mask, GestureResponder.bindEvent's geometric branch gated on
    GestureContainerFeature, ContentResponder, ViewTransform convert). The ONLY gap: no geometry-carrying
    leaf responder (RendererLeafView.makeLeafView emitted only a displayList, never a viewResponder) →
    gestures had empty children → empty mask → structural fallback.
    IMPLEMENTED (pieces 1+2+3):
      1. RendererLeafViewResponder (RendererLeafView.swift): a ViewResponder carrying the leaf's global
         frame; containsGlobalPoints sets mask bits for points inside the frame. Emitted from makeLeafView
         via a LeafViewRespondersRule (Attribute<[ViewResponder]>) when inputs.preferences.requiresViewResponders,
         using inputs.animatedPosition()/animatedCGSize() (the same geometry the display list draws with —
         position is the global origin the event globalLocation is in).
      2. GestureResponder.containsGlobalPoints (GestureViewModifier.swift): keep only AnyGestureResponder
         descendants in the result.children, so ViewResponder.hitTest STOPS at the gesture (not its content
         leaf) and returns the gesture — required because ViewGraph.sendEvents needs an AnyGestureResponder.
      3. Flip GestureContainerFeature.isEnabled→true (CustomFeature.swift); gate the location-blind fallback
         in EventBindingManager.bindResponder so a geometric miss returns nil (no first-gesture fallback).
    VERIFIED (WANDR_DEBUG_SYNTH_TAP_XY, now down+up): the hint-Text gesture's leaf frame = (39,95,422,22);
    synth-tap at (250,106) INSIDE → leaf hit=true, maskRaw=1, bound, TAP-FIRED=1; synth-tap at (200,300)
    OUTSIDE → hit=false, geometric-miss, no fire. Location discrimination works.
    KEY METHOD LESSON (cost ~6 build cycles): guest `print()` to stdout is block-buffered and LOST on
    SIGTERM/pkill — my HITTEST/GCHILDREN diagnostics read as 0 (phantom). Use `_gtrace` (fputs stderr +
    fflush) for any diagnostic you'll observe after killing the host. (Same class as the G4 abort-no-flush.)
    DEFERRED (not blocking firing or the migration):
      * PIECE 4 — global→local coordinate conversion. The injected MouseEvent.location == globalLocation
        (WandrApp.swift); nothing converts it into the gesture's view-local space, so Value.location /
        startLocation are reported in GLOBAL coords, not .local as SwiftUI specifies. Firing is unaffected
        (1-3 decide routing). DragGesture.translation is a DELTA → coordinate-space-invariant for
        untransformed views → board-swipe migration is correct without it. Per-view taps don't read the
        absolute location (hit-testing already routes them). Matters only for gestures using the absolute
        local point (e.g. SpatialTapGesture location, drawing). Machinery exists: ViewTransform.convert(.spaceToLocal).
      * PIECE 5 — transform-aware hit regions + multi-gesture ARBITRATION. TWO gestures on ONE view (the
        probe's hint Text has BOTH .onTapGesture and DragGesture) → geometric binding picks the FIRST and
        starves the other (no simultaneous/exclusive arbitration; LayoutGestureChildProxy stubbed). The
        gameplay migration (Part C) puts gestures on SEPARATE views, avoiding this. Harness: WANDR_DEBUG_SYNTH_TAP_XY / WANDR_DEBUG_SYNTH_DRAG_XY
    set the synth point (lib.rs). NEXT: Part C — board swipe→DragGesture + header/dialog→located taps, then
    retire the hand-rolled onPointer routing.

  METHOD / TOOLING (reusable, committed):
    * Fast symbolized backtrace, NO device round-trip / NO manual click:
      WANDR_DEBUG_SYNTH_TAP=1 (host hook in lib.rs, sibling of synth-key) fires a synthetic
      pointer-down at frame 2 in the desktop `--app` window. With WANDR_DEBUG_INFO=1 AND a
      component built with the NAME SECTION KEPT (run `wasm-tools strip` ONLY — SKIP the
      `strip --delete '^name$'` step), the trap prints a fully symbolized wasm backtrace on
      stderr. This pinned both G1 and G2.
    * Desktop GL present on WSLg drops after a few frames ("Connection reset by peer") — hence the
      frame-2 synthetic tap rather than a clickable window. Run the host detached, grep the log
      for "synth-tap FAILED" then the backtrace; kill the looping process afterward.
    * Demo probe: one .onTapGesture on the hint Text logs "TAP-FIRED"; the demo's on_pointer mirrors
      down/up into wandrSendPointer (@_spi(WandrRenderer) SPI on WandrApp).
    * Temp diagnostics still in tree (revert when G2+ done): #if os(WASI) [SRTRACE] in Compute
      StatefulRule.swift; [GTRACE]/[WANDR] in OpenSwiftUI GestureGraph/EventBindingManager/ViewGraph/
      GraphHost/WandrApp. The synth-tap + named backtrace supersedes these.

  STATE: gesture subsystem committed WIP — OpenSwiftUI bc9cdec4 (impl) + 2fa39812 (G1 fix);
  Compute 739cabe (traces) + 7bfeb9b (size-precondition diag); main 327a31d0 (synth-tap harness).
  Safe fallback: working manual-input 2048 build at main 92b45e3f. Device deploy pipeline =
  build-openswiftui-demo.sh -> wasm-tools component new --adapt -> WANDR_AOT_TARGET=
  aarch64-linux-android --install -> adb push -> wandr-arbiter launch.
================================================================================
