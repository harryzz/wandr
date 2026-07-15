---
name: reference-openswiftui-immortal-fix
description: "OpenSwiftUI-on-wasm 2048 demo — the aarch64 device \"0.42 miscompile\" was the cross-module foreign-ref over-release; immortal storage fixes it; animation+transitions now work clean + device-verified"
metadata: 
  node_type: memory
  type: reference
  originSessionId: 05cfcba8-822f-4c0c-a2b5-89e123d62b5e
---

OpenSwiftUI/Compute on wasm (repros/swift-canvas-spike, /tmp/Compute + /tmp/OpenSwiftUI): the
eleev/swiftui-2048 demo now runs CLEAN with **animation + transitions ON**, desktop (x86 JIT) AND
device (Pixel 2 XL aarch64 cross-AOT), 2026-06-25.

**Debunked:** the device `0.42` SIGSEGV was NOT a "wasmtime aarch64 Cranelift miscompile" (a prior
session's wrong conclusion). It was the cross-module foreign-reference **over-release** — off-Apple
there's no `objc_bridge` to unify Swift ARC with the CF refcount, so `IAG_SWIFT_SHARED_REFERENCE`
retain/release is asymmetric (`_ViewList_Subgraph.deinit`/`ItemInfo` array-destroy frees a storage
the live graph node still references) → over-release → double-free/UAF. The float `0.42`
(`0x3ed70e9c`) was that value reused where the freed Subgraph storage pointer had been.

**SURVIVAL FIX (superseded 2026-06-28):** make the CF storage **immortal** — `IAGSubgraphRetainRef`/
`IAGSubgraphReleaseRef` no-ops on wasm. Eliminated over-release/double-free/UAF for every Subgraph ref at
once. Tradeoff = bounded leak (the CF wrapper never freed). This was a wasm survival hack, NOT faithful AG.

**FAITHFUL FIX (2026-06-28, bug #14, replaces immortal on wasm — in swift/OpenSwiftUIProject/Compute):**
proved by a shadow-refcount instrument that the current ARC shape alone releases-to-zero WHILE the subgraph
is still alive (raw-pointer graph ownership; ARC refs transient) -> a real free there = UAF (exactly why
immortal was needed). Adding a **graph-alive self-ref** (extra `CFRetain` at `IAGSubgraphCreate2`, released
at `Subgraph::clear_object` = true death) keeps refcount>=1 for the whole lifetime -> premature=0, and the
ARC hooks made REAL (`CFRetain`/`CFRelease`) free the storage once dead AND unreferenced. Gate
`IAG_CF_STORAGE_SWIFT_MANAGED` = `__APPLE__ || __wasi__`. Reentrancy safe (self-ref => finalize only fires
after storage->subgraph nulled). VERIFIED: wasm suite 15/15 + storage actually freed (oagmemory 6000/6000
live=0; oagteardown 7051/7052) vs immortal=0-freed; linux 15/15 unchanged; 2048 demo still reaches frame #14
with the SAME #12 OpenSwiftUI crash (no new UAF). LINUX stays immortal (test-only platform; faithful there
needs the foreign-ref import = path-A C++-interop, blocked). Full ledger = WASM-PORT-LOG.md "#14".
Opt-in proof: env `IAG_STORAGE_LOG=1`. NOTE the band-aids below were already removed pre-immortal.

(Historical) The immortal era let me REMOVE all band-aids (from_cf liveness guard, 11 softened
"accessing invalidated subgraph" preconditions, DynamicLayoutViewChildGeometry offscreen hack, the
DynamicContainer.swift:453 isValid guard) — all dead code once refcounting is correct.

**Transitions (B3):** `supportsViewTransitions: true` + fixed an upstream constant-index bug
(`DynamicContainer.swift` ~line 440: `displayMap[validCount]` → `displayMap[validCount + index]`;
only reachable when removedCount!=0 = transitions on).

Zone-zeroing: ✅ DONE (2026-07-15 confirmed) — `ComputeCxx/Data/Table.cpp:57`
`memset(region, 0, initial_size); // wasi emulated mmap is NOT zeroed` on the initial region, and
`grow_region` (:121) zeroes the grown tail. Do NOT re-file this as a "still-needed root" — it is fixed.
Same wasm binary runs on wasmtime on ALL platforms (linux/windows/device), so there is no
"linux-mmap-zeroes / wasi-doesn't" platform split at the guest level. Remaining suspected root:
Subgraph/Graph member inits.

Method that cracked the original UAF: per-storage over-release trap (trap AT the ReleaseRef that drops a
long-lived storage to rc0) → DWARF backtrace named `_ViewList_Subgraph.deinit`. See
repros/openswiftui-wasm/RESUME.md (top) + the 7hr worklog /tmp/wandr-7hr-worklog.md.
Supersedes the band-aid era of [[reference_swift_openswiftui_wandr]].

## 2026-07-15 — INTERMITTENT node-level UAF still present under heavy real play
The eleev 2048 demo, played hard (many moves + menu/modal/settings/about/new-game churn), crashes
**intermittently on every platform** (same wasm), NOT the Subgraph-storage over-release this memory
fixed — a **node/attribute-level** dangling reference. Two observed surfaces, same family:
- `DynamicAnimationListener.animationWasRemoved → Attribute.invalidateValue → IAGGraphInvalidateValue →
  Graph::value_mark → propagate_dirty → IAG::Node::state()` → **OOB read** (animation removed invalidates
  a target attribute whose node was already freed/recycled).
- `GestureGraph.sendEvents → Attribute.setValue → Graph::value_set_internal` → `&type.value_metadata() !=
  &metadata` (reads garbage on a recycled node) → `precondition_failure` → `metadata::name()` →
  `swift_getTypeName` → wasm demangler `abort()` (Demangler.cpp:373) → component poisoned ("cannot enter
  component instance" flood). The demangler abort is COLLATERAL; the mismatch is the real fault. NB the
  error path calls the demangler and so tells us nothing — make it demangler-free before diagnosing.
Root: a freed AG node's slot is reused, then something still holding its `AttributeID` invalidates/sets
it. Intermittent = depends on reuse timing; needs heavy churn to hit. NOT caused by the 2026-07-15
gesture work ([[reference_openswiftui_gestures_offapple]]) — both crash paths are outside it (that work
only adds nodes/churn, which could nudge frequency).

TRACED (2026-07-15) — crash B mechanism: `DynamicAnimationListener.animationWasRemoved`
(`OpenSwiftUICore/Layout/Dynamic/DynamicContainer.swift:215`) DEFERS `asyncSignal.attribute?.invalidateValue()`
(`asyncSignal: WeakAttribute<Void>`) into `viewGraph.continueTransaction{…}`. The weak-liveness gate
`WeakAttributeID::expired()` (`ComputeCxx/Attribute/AttributeID/WeakAttributeID.cpp`) already applies the
`[#12]` rule — alive iff `zone_info.zone_id() == _seed && !zone_info.is_deleted()`. But it returned
false (alive) for a ref whose node resolves to a WILD pointer (`0x5ccc768` ≫ region end `0x3130000`). So
`expired()` PASSED yet the node is garbage → two residual edge cases: (1) **zone-id `_seed` COLLISION** —
a freed subgraph's zone-id reused by a live (not-deleted) subgraph, so the seed check spuriously passes;
or (2) a teardown path that frees the subgraph WITHOUT setting the deleted-bit (`mark_deleted` not called
there — the `[#12]` comment flags this was once dead code). The engine primitive (`expired`) is correct &
test-covered; the gap is teardown-ordering / zone-reuse the suite doesn't exercise, driven by how
OpenSwiftUI tears down animated subgraphs. NEXT: at the deref sites (`propagate_dirty` initial-node access
~Graph.cpp:1815-1820, `value_mark`), RANGE-check the resolved node ptr vs the region before deref +
mirror the `[#12]` `contains_subgraph` guard; on invalid, log the identifier/seed/zone-id and skip (or
trap demangler-free) — that both stops the OOB and reveals whether it's seed-collision vs missing
mark_deleted. Also make `value_set_internal` (Graph.cpp:1710) print pointers not `metadata.name()`
(avoids the wasm demangler `abort()` that currently destroys crash-A's info).

## 2026-07-15 (later) — per-site deref guards are WHACK-A-MOLE; it's pervasive corruption
Added Compute guards (Graph.cpp): value_set_internal non-fatal+demangler-free (crash A);
propagate_dirty initial-node + loop wild-output range guards (crash B) — the [#12] loop guard had a
hole: it only `continue`d for an IN-RANGE offset with a deleted subgraph, so a WILD offset
(`>= ptr_max_offset`, = `_vm_region_size+page`) made the outer `if` false and fell through to the OOB
deref; fixed to skip wild offsets FIRST. RESULT: crash B stopped recurring but the crash just MOVED to a
4th surface — `IAG::attribute_view::begin() → Subgraph::update` (OOB `0x22e775c0`). So there are ≥4
surfaces (A value_set, B propagate_dirty, C Map→EnvironmentValues `swift_retain`, D
attribute_view/Subgraph::update), ALL reading wild pointers far past the region — pervasive graph-memory
corruption, NOT a single bad deref. Per-site guards can't win. Wild addrs vary by surface (D recurs at
`0x22e77xxx` — also the very first crash seen this session; B gets `0x5exxxxx`; C `0x20080134`).
REAL FIX = find the CORRUPTION SOURCE (what writes a wild pointer into a graph node/edge/attribute-list
slot). Technique: a write-watchpoint / poison-on-free on the graph zone, or bisect what produces
`0x22e77xxx`, NOT more deref guards. This is the port's central hard problem = dedicated deep session.
The 2026-07-15 gesture/ABI work ([[reference_openswiftui_gestures_offapple]]) is unrelated & shipped fine;
this UAF predates it and surfaces only under heavy real play the old "reaches frame #14" tests never hit.

USER HYPOTHESIS (strong, narrows the hunt): the app was played for HOURS with NO crash back when it was
board+tiles ONLY — no side menu, no modals, no Settings/About, no buttons/rounded-rects. The crash
appeared once those DYNAMIC elements were added. Fits the diagnosis exactly: the board is a STATIC view
tree (tiles animate position but the container never appears/disappears), so it barely churns subgraphs.
The new elements — SideMenuView (slide in/out), BottomSlidableModalModifier (game-over/reset), and the
FactoryContentView `switch selectedView` game/settings/about SWAP — are DynamicContainer-backed views that
get created+destroyed WITH transitions as you navigate. That create/destroy-with-transition churn is
exactly the `DynamicAnimationListener.animationWasRemoved → invalidateValue` teardown path crash B lives in.
So the new UI doesn't CREATE the bug — it's the first thing to heavily EXERCISE a latent subgraph-teardown
UAF the static board never triggered. NEXT-SESSION BISECTION: (1) confirm board-only stays stable long;
(2) then hammer ONE dynamic element at a time (open/close side menu ×N; open/close a modal ×N; swap
settings/about ×N) to see which triggers the wild pointer — that localizes the corruption to a specific
DynamicContainer teardown, a much smaller haystack than "the whole engine". Prime suspect = the transition
teardown in `Layout/Dynamic/DynamicContainer.swift` + the conditional view-swap subgraph lifecycle.

BISECTED (2026-07-15, source-level) — the trigger is `DynamicContainer.eraseItem`
(`Layout/Dynamic/DynamicContainer.swift:701`). It has two teardown paths gated on
`unusedCount < Adapter.maxUnusedItems`: POOL (`parentSubgraph.removeChild(subgraph)` — subgraph
detached but KEPT ALIVE, nodes survive → safe) vs INVALIDATE (`subgraph.invalidate()` → destroyed →
deferred-teardown window → dangling cross-subgraph edges → the crashes). CRUX: `maxUnusedItems`
defaults to **`.zero`** (`DynamicContainerAdaptor.swift:61`) and NO conformer overrides it, so
`unusedCount < 0` is always false → the pool path is NEVER taken → **every** dynamic-view removal
invalidates. Static board never removes a dynamic view → never invalidates → the hours-crash-free
behavior. Menu/modal/Settings-swap remove views constantly → constant invalidate → the wild-pointer
crashes. FIX OPTIONS (a genuine tradeoff, user's call): (A) complete the [#12]-style liveness guards at
the remaining graph-walk sites (propagate_dirty done; add Subgraph::update/attribute_view for crash D,
etc.) — keeps AG invalidate semantics, bounded whack-a-mole; (B) give wasm dynamic containers a positive
`maxUnusedItems` so removals POOL (keep alive) instead of invalidate — avoids the window at the root, but
the pool REUSES subgraphs (`unremoveItem` phase==nil) which is risky for heterogeneous swaps
(game↔settings↔about reuse a subgraph across different content); (C) on wasm make eraseItem's invalidate
a detach-only leak (removeChild, never invalidate, never reuse) — safe from dangling edges & reuse bugs
but leaks every removed subgraph (unbounded; big views leak fast). (A) is most semantics-faithful and
matches the existing guards; (B)/(C) are immortal-philosophy stopgaps.
