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

Still-needed real roots: `Data/Table.cpp` zone-zeroing (wasi mmap not zeroed), Subgraph/Graph member
inits. Method that cracked it: per-storage over-release trap (trap AT the ReleaseRef that drops a
long-lived storage to rc0) → DWARF backtrace named `_ViewList_Subgraph.deinit`. See
repros/openswiftui-wasm/RESUME.md (top) + the 7hr worklog /tmp/wandr-7hr-worklog.md.
Supersedes the band-aid era of [[reference_swift_openswiftui_wandr]].
