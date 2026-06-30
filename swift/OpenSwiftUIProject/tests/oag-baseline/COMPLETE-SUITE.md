# Compute (AttributeGraph) — COMPLETE test-suite plan

## Why this exists (the 11-day regression)

The eleev/2048 demo was **device-verified playing 10 moves on 2026-06-25**. By 2026-06-27 it renders one
frame and faults. Over those days every session changed Compute/OpenSwiftUI and **each change made it
worse**, yet `oag-baseline` stayed **green the whole time** — because it only exercised basic rules +
propagation and **never touched the code that was breaking** (existential comparison, page-seed/weak-ref
expiry, value-witness `destroy` of ref-holding values during teardown).

A test suite that passes while the thing it tests regresses is worse than no suite — it grants false
confidence. The fix is a **COMPLETE** suite: one that exercises **every Compute subsystem along its
wasm-divergent paths**, run on **linux AND wasm every commit**, so "worse each time" fails a test the
moment it happens, at the Compute layer, with no OpenSwiftUI in the picture.

Compute is a clean-room reimplementation of Apple's AttributeGraph, fused to the Swift ABI and its own
mmap allocator. Bugs concentrate where wasm diverges from Apple/Linux: the Swift value-witness ABI
(metadata, existentials, foreign-ref `Subgraph`) and the page allocator (recycle, grow, seeds). The
suite must hammer exactly those.

## Coverage matrix

| Compute subsystem | wasm-divergent risk | Existing | This session | COMPLETE-suite gap |
|---|---|---|---|---|
| Dataflow (rules, deps, propagate) | low | `oagdataflow` ✅ | — | adequate |
| Subgraph churn (volume) | med | `oagchurn` ✅ | — | adequate for volume; not for value shape |
| Dynamic-list identity (ForEach) | high | `oagforeach` ✅ | — | add struct-held **foreign-ref Subgraph** across cycles (see ⑦) |
| Render walk | low | `oagrender` ✅ | — | adequate |
| **Value comparison engine** | **high** | none | **`oagcompare` ✅** (existential freeze + recursion) | **enums/CoW, indirect, heap-objects, big values, compare_bytes** (①) |
| **Subgraph teardown / vw_destroy** | **high** | none | **`oagteardown` ✅** (class values) | **ref-holding values, deferred invalidation, invalidate-during-update** (②) |
| **Zone / page memory** | **high** | none | none | **`oagmemory` — page recycle, grow, persistent buffers, seeds** (③) |
| **Weak / indirect references** | **high** | none | none | **`oagweakref` — WeakAttributeID seed expiry, indirect→invalidated source** (④) |
| **Graph update machinery** | med | partial | none | **`oagupdate` — input-recursion, update-of-invalidated, cycles** (⑤) |
| **Attribute/Node values** | med | partial | none | **`oagvalues` — indirect/large/self, init vs assign vs destroy** (⑥) |
| **Swift ABI bridge** | **highest** | none | none | **`oagbridge` — foreign-ref Subgraph in struct fields, metadata-pool stability** (⑦) |

`oagcompare` and `oagteardown` were added this session. `oagcompare` would have **caught both** of the
session's worst crashes (the freeze AND the existential-recursion shadow-stack overflow). `oagteardown`
passes but is **incomplete** — see ②.

## Missing test targets to implement (prioritized by what actually broke)

### ① `oagcompare` — EXTEND (comparison engine)  [partly done]
Done: existential `any P` freeze + self-nesting recursion. **Add:**
- **Enums** (the destructive-projection / CopyOnWrite path in `Compare.cpp`): optionals, multi-payload
  enums, enums nested in structs; mutate→recompare must be non-destructive (the value must survive being
  compared) and correct.
- **Heap objects** (`compare_heap_objects`, CoW-clear path): class-typed fields compared by identity vs
  by value; same-instance short-circuit; different-instance deep compare.
- **Indirect / boxed** values: out-of-line existentials, copy-on-write buffers.
- **Large values** (>0x10 — the `alloc_bytes` path) and **mixed trivial/non-trivial** structs.
- **`compare_bytes` 8-byte path on wasm32**: structs whose 4-byte pointer fields straddle the 8-byte
  compare stride (the wasm32 pointer-packing the prior session hit) — false-change detection guard.

### ② `oagteardown` — EXTEND (teardown / value-witness destroy)  [partly done]
Done: class-valued attributes + cascade, 3000×. **Add (this is where the *desktop* crash actually lives —
a simple `Box` did NOT reproduce it):**
- **Values holding a `Subgraph`/foreign ref** (mirrors `DynamicViewList`/`ViewList.Subgraph`, the
  `vw_destroy@0xfffffff8` crash): an attribute whose value struct holds a child `Subgraph`, torn down →
  the destroy must release it without corrupt metadata. **← highest-value missing test.**
- **Deferred invalidation**: `invalidate` while `withoutSubgraphInvalidation`/an update is active; verify
  the teardown flushes at the right boundary and not mid-pass.
- **Invalidate-during-update**: invalidate a child whose attribute a sibling reader resolved THIS pass
  (the move-2 scenario) — the page must not be freed under the live read.
- **Re-insertion**: remove → re-add a child subgraph (didReinsert) and keep reading.

### ③ `oagmemory` — NEW (zone / page allocator)  [the page-recycle/grow family]
- **Page recycle**: allocate many small values (`alloc_bytes_recycle` free-list), free, re-alloc; verify
  no aliasing / stale reuse.
- **Region grow** (`grow_region`): force the zone past its initial reservation; on wasm this historically
  left OLD/NEW divergent copies — verify pointers stay valid across a grow.
- **Persistent buffers** (`alloc_persistent` / `_has_indirect_value`): large/indirect attribute values;
  verify the malloc-zone buffer survives + frees correctly on teardown.
- **Page seeds** (`raw_page_seed`): the per-page seed must change exactly when a page is recycled and not
  otherwise (the false-expiry the move-2 weak-ref bug rode on).

### ④ `oagweakref` — NEW (weak / indirect references)  [the move-2 "invalid source attribute" family]
- **WeakAttributeID expiry**: weak ref to an attribute whose subgraph is invalidated → expires; to a LIVE
  attribute → must NOT expire (the `subgraph_id` vs `zone_id` divergence that caused false expiry).
- **IndirectNode resolve** + **MutableIndirectNode** redirection.
- **Indirect → invalidated source**: read a weak-sourced indirect after its source's subgraph dies →
  yields nil (not a hard `add_input` precondition); cleanup on `remove_indirect_node`.
- **OptionalAttribute** over an invalidated source.

### ⑤ `oagupdate` — NEW (graph update machinery)  [the move-4 OOB family]
- **Input-recursion dispatch**: a node pushed while valid, then a deeper update invalidates its subgraph →
  it must NOT be dispatched stale (the move-4 OOB funcref).
- **Update-of-invalidated subgraph** skipped at the dispatch site.
- **Cyclic edges** detection; deep (1000+) update recursion bounds; `mark_changed` edge propagation.

### ⑥ `oagvalues` — NEW (Attribute/Node value lifecycle)
- Indirect values (`_has_indirect_value`), large values, `self`/body; `initializeWithCopy` vs
  `assignWithCopy` vs `destroy` for trivial, class-holding, existential, and enum value types.

### ⑦ `oagbridge` — NEW (Swift ABI bridge — the deepest, the 11-day root)  [immortal-storage family]
- **Foreign-ref `Subgraph` held in STRUCT fields across many cycles** (the `objc_bridge`-empty-on-wasm
  problem): a long-lived struct holding child-subgraph refs; allocator churned between cycles; the storage
  must stay alive (no UAF) and refs must stay identity-stable. This is the immortal-storage root that the
  prior working build depended on and that regressed.
- **Metadata-pool stability**: run many graph ops (compare/update/teardown) and assert a shared type's
  metadata/VWT is never zeroed (a software write-tripwire guard for the existential-recursion class).

## How to run (every commit, both platforms)

- linux native: `OPENATTRIBUTEGRAPH_USE_LOCAL_DEPS=1 OPENATTRIBUTEGRAPH_OPENATTRIBUTESHIMS_COMPUTE=1 swift build && swift run <target>`
- wasm: `bash build-wasi.sh` then `wasmtime run --env SWIFT_DETERMINISTIC_HASHING=1 -W max-wasm-stack=8388608 .build/wasm32-unknown-wasip1/debug/<target>.wasm`
- A bug that reproduces on **wasm but not linux** = a wasm-port defect (the common case); both green = real.

## Using the suite to find the 11-day regression (bisect)

Once ②/③/④/⑦ exist (the families that actually broke), `git bisect` the OpenSwiftUI + Compute history
against them on wasm. The strong suspects to land on: the `0f4f20bf` WasiClosureShim consumer integration
(introduced the freeze) and the CF-bridging migration that replaced the prior **foreign-ref `Subgraph`**
approach the device-verified build used. The bisect tells us whether to **fix forward** or **revert to the
working approach** — instead of patching symptoms, which is what made it worse each time.

---

## Implemented coverage + findings (this pass)

The suite now has **15 targets**, run on **wasm AND linux native**. Beyond the bug-family tests
(`oagvalues`, `oagcompare`, `oagupdate`, `oagmemory`, `oagweakref`, `oagteardown`, `oagbridge`) the
functional-coverage half is in: `oaggraph` (graph services), `oagsubgraph` (subgraph services),
`oagattr` (attribute ops), `oagrules` (StatefulRule / Map), on top of the original
`oagdataflow`/`oagchurn`/`oagforeach`/`oagrender`.

### Crashers found (none seen by the old baseline)
- **`oagupdate` → OOB memory on WASM, passes on linux.** ✅ **FIXED (2026-06-28).** Root cause was *not*
  the update machinery's logic — it was the **guest shadow stack**. The wasi link-time default
  (~128 KiB) is far below the host's wasm call-stack budget (`wandr-host` `max_wasm_stack`=4 MiB /
  `async_stack_size`=8 MiB) and the 8 MiB native thread stack the linux build runs on. AttributeGraph
  recurses on first evaluation (lazy input-edge creation: `input_value_ref_slow` → `update_attribute`
  → `update` → rule → `input_value_ref_slow` …), so a deep attribute chain overflowed the tiny shadow
  stack into the heap and faulted with a wild pointer. Controlling test: depth 50 OK, 200+ OOB; no zone
  grow (`IAG_LOG_GROW`). Fix: (a) `-Xlinker -z -Xlinker stack-size=8388608` in `build-wasi.sh` (8 MiB
  guest/host/native parity); (b) `UpdateStack::update()` made iterative (`goto restart_frame`, was
  `return update()`) so the steady-state re-eval path no longer grows the C++ stack at all. Suite green
  on wasm AND linux.
- **`oagteardown` → crash (Aborted) on LINUX, passes on wasm.** ✅ **FIXED (2026-06-28).** Mis-titled
  originally — it is *not* a teardown/CFRelease-reentrancy defect. The abort is at `value_ref_checked`
  "attribute being read has no value", reached from the nested-tree / deferred-read block (not the 3000
  teardown rounds, which all pass). Root cause: **Subgraph storage lifetime off Darwin.** The CF storage
  is ARC-bridged (`objc_bridge(id)`) only on Apple; off Darwin Swift does NOT retain the handle/struct-held
  refs, so the CF refcount is the sole owner. The immortal-storage guards keyed on `__wasi__`, so **linux**
  still ran `CFRelease` (in `IAGSubgraphSetCurrent`/`Subgraph::clear_object`) — freeing a subgraph the live
  graph still referenced. Proof: 3 value-attributes created in 3 separate subgraphs all aliased to the same
  node id (536) on linux because each freed subgraph's page was recycled by the next; retaining the handles
  → distinct ids. Note the `objc_bridge` attribute condition is *active on non-Apple clang too* (it parses),
  so it is NOT a valid predictor — `__APPLE__` is. Fix: new single-source predicate
  `IAG_CF_STORAGE_SWIFT_MANAGED` (= `defined(__APPLE__)`) in `IAGBase.h`; the two subgraph CFRelease gates
  now use it → storage immortal on BOTH linux and wasm (bounded CF-wrapper leak; the IAG::Subgraph node is
  still freed by the graph on invalidate). Suite green both platforms except oagrender (below).
- **`oagrender` → flaky Sig11 on LINUX** (passes on wasm). ✅ **FIXED (2026-06-28).** Root: `Subgraph.forEach`
  passed its **non-escaping** `body` to the swiftcall `IAGSubgraphApply`, which wraps it in a
  `ClosureFunctionAV` whose ctor `swift_retain()`s the closure context (ClosureFunction.h:24). A
  non-escaping closure's context is not a retainable heap object → `incrementSlow` derefs garbage →
  flaky crash (the deref address varied run-to-run = heap-layout sensitive). wasm dodged it via the
  plain-C `subgraphForEach` (no retain). Fix: wrap the non-wasm path in `withoutActuallyEscaping` so the
  retain/release pair operates on a valid context. oagrender now 20/20 on linux (was ~13% Sig11), 10/10 wasm.

### Real gaps found — FIXED 2026-06-28 (persistent-callback wasm ABI family)
Bugs (4) and (5) were ONE root: the **persistent swiftcc-`ClosureFunction` callback path is unusable on
wasm**. `__has_attribute(swiftcall)` is true under Swift's wasm clang, so the stored callback is a
swiftcall fn-ptr — but Swift emits a `@convention(c)` thunk for the closure, and firing the swiftcall
pointer via the C ABI traps `indirect call type mismatch`. (onUpdate just never fired before, so its
identical latent bug was invisible; addObserver fired on invalidation and trapped.) The synchronous
shims already dodge this with plain-C `Plain*` bodies (PlainApplyBody / PlainTypeIDGetter).
- **`onUpdate` / `onInvalidation`** ✅ — added `Graph::Context::PlainUpdateCallback` /
  `PlainInvalidationCallback` (wasm storage) + plain-C `IAGGraphSetUpdateCallbackC` /
  `IAGGraphSetInvalidationCallbackC`; `WasiClosureShim.onUpdate/onInvalidation` use them. **Plus** a
  second wasm bug surfaced once onUpdate fired: `Table<uint64_t,Context*>::for_each` (HashTable.h)
  type-puns the typed callback to the untyped `(const void*…)` signature — i64 key ≠ i32 on wasm →
  `indirect call type mismatch` inside `Graph::call_update`. Fixed with a wasm trampoline in
  `Table::for_each`. onUpdate now **fires on wasm AND linux** (verified via `setNeedsUpdate()`).
  onInvalidation uses the identical mechanism (cross-context trigger not unit-tested; fixed by symmetry).
- **`Subgraph.addObserver`** ✅ (was `signature_mismatch`) — added `Subgraph::PlainObserverBody` +
  plain-C `IAGSubgraphAddObserverC`; the swiftcc `IAGSubgraphAddObserver` + its stale 2-arg silgen are
  gated to non-wasm. Observer now **fires on wasm AND linux**.
- **keyPath subscript `attr[keyPath: \.m]`** ✅ **re-verified working (2026-06-28)** on wasm AND linux —
  the recorded "traps non-direct attribute" no longer reproduces. Both spellings share the same
  `subscript(keyPath:)`; for stored-property keyPaths `MemoryLayout.offset(of:)` resolves → direct
  `unsafeOffset` (no trap), and for computed/no-offset keyPaths → `Focus` indirect, which reads fine.
  The "non-direct attribute id" precondition is only reachable via info/mutate ops
  (`IAGGraphGetAttributeInfo`/`MutateAttributeC`) on an indirect attribute, not value reads. `oagattr`
  now asserts the keyPath spelling directly (workaround removed); passes both platforms.

### Intentionally unimplemented on wasm (`fatalError("not implemented")` — not bugs, not tested)
`Graph.print`, `archiveJSON`, `graphvizDescription`, `printStack`, `stackDescription`,
`startProfiling`/`stopProfiling`/`markProfile`/`resetProfile`, `addTraceEvent` — introspection /
profiling / tracing. Calling any aborts the process, so the suite records them as N/A rather than exercising them.

### Still not covered (next round of functional targets)
External / Focus attributes; RuleContext direct; PointerOffset / `applying(offset:)`; breadthFirstSearch;
explicit `addInput(options:token:)` edges; AsyncAttribute; DefaultRule / cycle detection; tree elements
(`beginTreeElement`/`addTreeValue`); subgraph flags + cross-context children + multiple parents;
`withMainThreadHandler` firing; custom-Equatable compare dispatch.
