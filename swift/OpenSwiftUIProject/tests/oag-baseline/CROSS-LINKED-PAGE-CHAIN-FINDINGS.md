# Cross-linked subgraph page-chain UAF — findings + fix proposal

**Date:** 2026-07-15
**Status:** root mechanism pinned by measurement; exact corrupting operation not yet located (next step below)
**Detector:** deterministic headless repro of the eleev/swiftui-2048 intermittent AttributeGraph crash — see [[reference_openswiftui_headless_uaf_repro]]

## Goal discipline (per [[feedback_compute_goal_and_working_rules]])

The goal is Compute correctness, not "the demo survives." Every step below is: **read the real source
path first, then measure to confirm/refute a specific reading** — never patch-and-cycle, never mask a
crash site with a defensive guard. Two prior hypotheses were investigated and **disproven by measurement**
(recorded below so they are not re-tried). The immortal-graph flag exists **only** as a diagnostic
A/B switch (default OFF) — it is explicitly **not** a candidate fix; it masks the symptom (leaks freed
pages instead of freeing them) rather than fixing why a live subgraph loses a page.

## The deterministic repro

`repros/swift-canvas-spike/Sources/T2iles/WandrHeadless.swift` (`#if WANDR_HEADLESS`, temporary — remove
after the fix lands). Renders the **real** `CompositeView` headlessly (text-sink, no host/canvas), drives
it through the **real** input shim (`wandrSendPointer`), and repeats: swipe the board a few times → **reset
the game (header new-game button → tap "Ok")** → navigate **Settings → About → game** via the hamburger
menu. This exact sequence — matching the user's real report ("reset the game, then tap Settings") — crashes
**deterministically, within 0–4 rounds, on essentially every run.** Pure navigation without a reset is
clean (survives 400+ rounds); the reset is the necessary trigger.

Build: `bash repros/swift-canvas-spike/build-headless.sh`. Run:
```
wasmtime run -W max-wasm-stack=8388608 \
  --preload "wasi:canvas/draw@0.0.2=stub_0.wasm" \
  --preload "wasi:canvas/layout@0.0.2=stub_1.wasm" \
  --preload "wasi:canvas/types@0.0.2=stub_2.wasm" \
  --invoke 'wasi:input-handlers/frame-handler@0.0.2#on-frame' \
  .build/wasm32-unknown-wasip1/debug/T2iles.wasm 0
```
(the three `--preload` stub modules satisfy ~25 dead `wasi:canvas` imports the reactor links but never
calls under headless — generation script referenced in the repro memory file.) Add
`WASMTIME_BACKTRACE_DETAILS=1` for full file:line backtraces (the guest carries complete DWARF).

This same crash was independently confirmed on the **real Windows GUI host** after ~10 min of play:
`Assertion failed: _value > KindMask (AttributeID.h:93)`, followed by every subsequent frame trapping
`cannot enter component instance` (the guest abort poisons the wasmtime component instance — this is why
the app "freezes" rather than cleanly exiting). Same corrupted-AttributeID family, just landing at the
opposite (near-zero) end from the headless repro's typical `0x22e_xxxx` wild-high faults.

## Chain of measurement (each step: read source → instrument → run → record verdict)

### 1. Byte-level free-list reuse — RULED OUT
`Zone.cpp`: `alloc_bytes_recycle`/`realloc_bytes` write freed byte-ranges into `_free_bytes` for reuse
within a zone. Disabling only this (never recycle bytes, still free/reuse whole *pages* normally) — **still
crashed 3/3**, same `0x22e57b90`-family fault. Byte-level free-list is not the mechanism.

### 2. Page-level reuse (`zone::clear` → `table::shared().dealloc_page_locked`) — CONFIRMED AS THE CHANNEL
Extending the same disable to **page**-level reuse (leak pages instead of returning them to
`table::shared()`) flips the outcome: **A/B on the identical binary, one env var
(`WANDR_IMMORTAL_GRAPH`)**:
- `=0` (normal reuse): crash at round 0, exit 134, wild `0x22e57b90`.
- `=1` (page-level leak): survives 20+ rounds, still running when killed.

This isolates the UAF to **reuse of a torn-down subgraph's page memory** by a subsequent allocation. (This
flag stays in the tree as a diagnostic only — default OFF — never as a shipped fix; see Goal discipline.)

### 3. "Stale child left in a parent's `_children`" — RULED OUT
Read `Subgraph::update`'s child-traversal push (`Subgraph.cpp` ~line 738-744): it dereferences
`child.subgraph()` (a raw pointer from `_children`) with no liveness check before pushing it onto the
traversal stack, unlike the `from_cf`-guarded pop path. Hypothesis: a stale child (never unlinked from a
live parent's `_children`) is derefed here. Instrumented a `contains_subgraph` check at that exact push
site — **fired 0 times** across a full crashing run. Every child ever pushed was a registered, live
subgraph. This hypothesis is dead.

### 4. "Reentrant `invalidate_and_delete_` from the CF finalizer" — RULED OUT
`WASMTIME_BACKTRACE_DETAILS=1` + DWARF gave a full symbolic crash backtrace on one run:
```
_swift_release_dealloc <- swift_release <- CFRelease <- IAGSubgraphReleaseRef (IAGSubgraph.cpp:85)
  <- DynamicViewList value-witness destroy <- AttributeType::destroy <- Node::destroy (Node.cpp:102)
  <- Subgraph::invalidate_now (Subgraph.cpp:276) <- Graph::invalidate_subgraphs (Graph.cpp:292)
  <- Subgraph::update (Subgraph.cpp:759) <- GraphHost.finishTransactionUpdate <- GestureGraph.sendEvents
```
Read the subgraph-storage CF finalizer (`IAGSubgraph.cpp`, `subgraph_type_id()`'s `finalize` lambda):
```cpp
static auto finalize = [](CFTypeRef subgraph_ref) {
    IAGSubgraphStorage *storage = (IAGSubgraphStorage *)subgraph_ref;
    IAG::Subgraph *subgraph = storage->subgraph;
    if (subgraph) {
        subgraph->clear_object();
        subgraph->invalidate_and_delete_(false);   // <- could this re-enter mid-teardown?
    }
};
```
Hypothesis: destroying a `DynamicViewList` node inside `invalidate_now`'s own node-destroy loop releases a
held `IAGSubgraphRef`, dropping its refcount to 0, running the finalizer, which re-enters
`invalidate_and_delete_` **while the outer `invalidate_now` is still iterating** — corrupting the in-flight
teardown. Instrumented a reentrancy-depth counter (`g_iag_in_invalidate_now`, RAII-scoped around
`invalidate_now`) read from the finalizer. Result: **638 finalizer calls fired *during* an in-progress
`invalidate_now`**, but **100% of them found `subgraph == nullptr`** (i.e. `clear_object()` had already run
and nulled the back-pointer before this release) — so **none re-entered `invalidate_and_delete_`.** This
hypothesis is dead; the finalizer path is release-only in every observed case, not reentrant.

### 5. Page-ownership provenance — CONFIRMED, PINNED TO THE EXACT MECHANISM
Built a page-owner map (`Zone.cpp`): `wandr_set_page_owner(page_offset, zone_id)` on every page allocation
(`alloc_slow`), `wandr_clear_page_owner(page_offset)` on every page free (`zone::clear`). Checked at the
**exact crash site** — the top of `Subgraph::update`'s page loop, immediately before
`attribute_view::begin()` dereferences `page->bytes_list` (`Subgraph.cpp` ~line 696): does this page's
current owner match the subgraph currently iterating it?

Result — fired right before the crash, every crashing run:
```
Subgraph::update iterating page off=271360 of subgraph_id=296
  but page's CURRENT owner=837 (REUSED-by-another)
```
(also seen: `owner=~0/"untracked/WILD"` — a page offset that was never a real allocation at all, i.e. a
fully corrupted pointer, not just a reused one.)

**Disambiguation — is subgraph 296 itself a stale/reused `Subgraph*`, or genuinely live?** Added a
live-subgraph-by-id registry (populated in the constructor, erased in the destructor, keyed by `zone_id()`)
and cross-checked the iterated pointer against it. Verdict, consistent across every occurrence: **the
iterated subgraph IS the real, live, currently-registered object** (`live-obj-for-id == the pointer being
iterated`, every time). This is **not** stale-pointer reuse. It is case (b): **a genuinely live subgraph
has a page in its own `_first_page` chain that has been freed and handed to a different subgraph.**

**Free-history — who freed the page, and did the "live" subgraph free its own page or is this cross-zone?**
Added a free-history map (`page_offset -> {freeing_zone_id, sequence_number}`), recorded in `zone::clear`.
One crashing run: page `271360` — currently in **live** subgraph **296**'s chain — was **freed by subgraph
759** at sequence 2071, and its ownership now belongs to subgraph 837. Subgraph 296 is a third,
uninvolved, still-alive party.

### Conclusion of the measurement chain

**A page ends up simultaneously reachable from two different subgraphs' `_first_page`/`page->next` chains
— live subgraph 296's chain and (at some point) subgraph 759's chain. When 759 is torn down and its pages
are returned to the shared page pool (`zone::clear` → `table::shared().dealloc_page_locked`), that shared
page is freed and later reallocated to a third subgraph (837) — but subgraph 296's chain still links to it
(via a stale `page->next`), so 296 walks into memory now owned by 837 (or, in the wilder case, memory that
was never validly allocated at all).**

This is a genuine defect in Compute's subgraph/page ownership model: pages must be exclusively owned by
one zone at a time and unlinked from every referencing chain before being freed. Somewhere in the codebase
a page pointer is being **copied into (or left in) a second subgraph's chain** without transferring or
duplicating true ownership.

## FIXES APPLIED (2026-07-15)

Per direct guidance ("go straight to clear_object()"): re-reading `Subgraph::clear_object()` itself showed
it only manages the CF storage-wrapper's retain/release (no page/chain code at all) — but tracing ONE level
out from the exact code region it sits in (the same-context child-teardown loop in `invalidate_now` that
calls it) led straight to the real defect.

### Fix 1 — `Graph::remove_subgraph` missing an erase (Graph.cpp)

`Graph::_invalidating_subgraphs` (`Graph.h:117`, a plain `vector<Subgraph *>`) is a GRAPH-LEVEL deferred
queue: pushed to once, in `Subgraph::invalidate_deferred` (`defer_subgraph_invalidation`), popped once, in
`Graph::invalidate_subgraphs()`'s consumption loop, which calls `subgraph->invalidate_now(*this)` on each
popped raw pointer. **Nothing ever removed a stale entry** if that subgraph was torn down via a *different*
path first — specifically, as a same-context CHILD inside ANOTHER subgraph's `invalidate_now` (the loop
`clear_object()` sits in): that path calls `clear_object()`, pushes the child onto invalidate_now's own
LOCAL stack, and — within the SAME call — fully tears it down (frees its pages, `delete`s the C++ object)
without ever touching the GRAPH-level queue. `Graph::remove_subgraph` (called from BOTH teardown paths,
right before final deletion) already does this exact "strike from every index" cleanup for `_subgraphs`,
`_tree_data_elements_by_subgraph`, and `_subgraphs_with_cached_nodes` — but was missing the same erase for
`_invalidating_subgraphs`. A textbook erase-remove-idiom omission (the project's own documented bug class).

**Consequence when it fires:** a stale `Subgraph*` sits in the queue past its object's destruction. When
`Graph::invalidate_subgraphs()` later pops it and calls `invalidate_now()`, that memory — freed and, under
heavy allocation churn, already reused for a brand-new UNRELATED live subgraph — gets torn down BY MISTAKE:
its pages freed while every other structure still thinks it's alive. This exactly matches the
"REUSED-by-another WITH a recorded free-history" measurement class (e.g. `subgraph_id=296`'s page freed by
`759`, later owned by `837`).

**Fix:** add the missing erase, symmetric with the others already in the function:
```cpp
{
    auto iter2 = std::remove(_invalidating_subgraphs.begin(), _invalidating_subgraphs.end(), &subgraph);
    _invalidating_subgraphs.erase(iter2, _invalidating_subgraphs.end());
}
```
Added a matching **permanent, cheap regression guard** in `Graph::invalidate_subgraphs()`'s pop loop —
before the popped pointer is dereferenced at all, confirm it's still a live, registered subgraph
(`contains_subgraph` is a pure pointer search, never touches the pointee, so it's safe even if the pointer
is dangling):
```cpp
while (!_invalidating_subgraphs.empty()) {
    auto subgraph = _invalidating_subgraphs.back();
    _invalidating_subgraphs.pop_back();

#if defined(__wasi__)
    if (!contains_subgraph(subgraph)) {
        precondition_failure("stale entry in deferred-invalidation queue: subgraph %p is not a "
                              "live, registered subgraph (regressed cross-linked-teardown bug — "
                              "see tests/oag-baseline/CROSS-LINKED-PAGE-CHAIN-FINDINGS.md)",
                              (void *)subgraph);
    }
#endif
    subgraph->invalidate_now(*this);
}
```
This is the tripwire for the fix above regressing: if some future change reintroduces a path that tears a
subgraph down without removing its queue entry, this fires a hard, clearly-worded crash at the exact
moment it goes wrong — instead of the corruption silently surfacing somewhere unrelated later (which is
what made the original bug take so long to find). Zero cost when the invariant holds (one cheap pointer
search on an infrequent path); it is a genuine assertion, not a fallback, so it cannot mask a regression.

### Fix 2 — `table::alloc_page` multi-page scan control-flow bug (Table.cpp)

Testing fix 1 alone against the deterministic repro **did not** eliminate the crash — the regression guard
never fired, and the surviving crash pattern shifted to a DIFFERENT signature: `page-owner=~0 (untracked)`
with **no free-history at all** (`freed-by-id=(none)`), i.e. a page offset that was handed to a second zone
WITHOUT ever going through `zone::clear()`/`dealloc_page_locked`. That ruled out fix 1 as the sole cause and
pointed at the allocator itself.

`table::alloc_page`'s free-page scan (`Table.cpp` ~lines 187-213), for a multi-page request
(`needed_pages > 1`), checks each of the `needed_pages - 1` subsequent pages for availability. On finding
one already in use, it disqualifies the *candidate* bit (`free_pages_map.reset(candidate_bit)`) and
`break`s out of the inner `for` loop — clearly intending the outer `while` loop to retry with the next
candidate. But the very next line, **unconditionally**, set `found = true` regardless of which branch broke
the loop — so the disqualified, conflicting candidate got used anyway: `alloc_page` returned an
ALREADY-IN-USE page to a second, unrelated zone, silently, with no corresponding free ever recorded. That
page is now linked into TWO zones' `_first_page` chains simultaneously; whichever zone's writes land there
second corrupts the other's page header (`bytes_list`/`next`), and either surfaces as
`attribute_view::begin()` reading scrambled data or as the chain hopping to a downstream wild pointer
outside the region entirely (the `offset=536871016` / ~512MB "untracked/WILD" family).

**Fix:** track a separate `conflict` flag; only fall through to `found = true` if the scan completed with
no conflict:
```cpp
bool conflict = false;
for (int j = 1; j < needed_pages; j++) {
    ...
    if (_page_maps[next_map_index].test(next_page_index % pages_per_map)) {
        free_pages_map.reset(candidate_bit);
        conflict = true;
        break;
    }
}
if (!conflict && !found) {
    found = true;
}
```

### Result — VERIFIED

With both fixes applied: **6/6 runs survived all 60 rounds (moves + reset + navigation), exit 0, zero
memory faults, zero assertion failures**, versus **crash at round 0, every single run** (dozens of samples)
before the fixes. This includes a run on the fully cleaned tree (all diagnostic-only instrumentation
removed, only the two real fixes + the one permanent regression guard remaining) — the fix is not an
artifact of the diagnostic scaffolding.

The diagnostic-only instrumentation (page-owner map, free-history map, live-subgraph-by-id registry, the
per-page WANDR-PROV check, the disproven-hypothesis reentrancy counter and stale-child check, and the
`WANDR_IMMORTAL_GRAPH` bisection flag) has all been **removed** — it served its purpose (isolating and
confirming the mechanism) and is not needed going forward; this document + git history preserve how to
rebuild it if a similar bug class needs diagnosing again. What remains permanently in the tree:
- The two real fixes (`Graph::remove_subgraph`'s missing erase; `table::alloc_page`'s scan control-flow fix).
- One cheap, permanent regression guard in `Graph::invalidate_subgraphs()`'s pop loop
  (`contains_subgraph()` check before `invalidate_now`, `#if defined(__wasi__)`) that hard-fails with a
  clear message if this exact bug class (a stale queue entry surviving a subgraph's destruction) ever
  regresses.
- The `#if WANDR_HEADLESS`-gated deterministic repro itself (`WandrHeadless.swift`), which stays as
  permanent regression-test infrastructure per [[reference_openswiftui_headless_uaf_repro]].

## Proposal — how to close the gap (next concrete step)

The measurement chain has been symptom → owner-mismatch → live/stale disambiguation → free-attribution.
The one remaining question is **which specific operation performs the cross-link** — i.e. where does a
page pointer get written into a *second* subgraph's chain (or where does one subgraph's chain fail to be
fully rebuilt/severed when subgraphs are split, merged, or reparented).

**Proposed next measurement (chain-integrity check, not a fix):** at every point a page is linked into a
zone's `_first_page` chain — `alloc_slow`'s `new_page->next = _first_page; _first_page = new_page;` (both
branches) — assert/record that the page was not already linked to a **different**, still-live zone. Also
audit the structural candidates read but not yet instrumented:
- `Subgraph::invalidate_now`'s child-reparenting block (`Subgraph.cpp` ~lines 206-244), which walks
  `subgraph->_children` and rewrites `_parents`/`_children` on subgraphs belonging to the *same context* —
  this is the one place subgraph structure is actively rewired mid-teardown, and is the most likely site
  where a page-owning relationship could be misattributed if a `Subgraph` (not just its parent/child links)
  is treated as shared/moved between contexts.
- Anywhere a `Subgraph::update` traversal or a context/graph "shared" construction
  (`Graph(shared:)`, used by the earlier `oagdangling` repro to arm cross-context reentrancy) might cause
  two `Subgraph` objects to alias the same underlying page allocation, e.g. via a copy/move of `_first_page`
  that isn't also exclusive.

Once the exact write site is caught red-handed (the assert fires with a live stack trace showing the
operation that links page P into zone Z2 while it's still in zone Z1's chain, or fails to unlink it), the
real fix is: **that operation must either (a) not share the page at all (each zone's pages must be
disjoint), or (b) if legitimate ownership transfer, correctly unlink the page from the source zone's chain
at the same time.** This is a targeted, source-level Compute fix — not a guard, not a leak, not a
workaround — consistent with the project's binding rule to find and fix real Compute defects.

## Instrumentation currently in the tree (all `#if defined(__wasi__)`, all temporary — remove after fix)

- `Zone.cpp`: `g_iag_disable_recycle()` (env `WANDR_IMMORTAL_GRAPH`, default OFF, diagnostic-only A/B),
  page-owner map (`wandr_set_page_owner`/`wandr_clear_page_owner`/`wandr_get_page_owner`), page free-history
  map (`wandr_record_free`/`wandr_get_free_info`).
- `Subgraph.cpp`: `g_iag_in_invalidate_now` reentrancy-depth counter + `InvalidateNowGuard` RAII (kept —
  cheap, useful signal); live-subgraph-by-id registry (ctor/dtor hooks); the `WANDR-PROV` provenance check
  in `Subgraph::update`'s page loop (the disproven `contains_subgraph` child-push check has already been
  left in place too — harmless, 0-cost when it doesn't fire, useful as a permanent canary).
- `IAGSubgraph.cpp`: `WANDR-MEASURE` log in the CF finalizer (reentrancy check — proved release-only).

None of these change behavior when `WANDR_IMMORTAL_GRAPH` is unset (default) except for the `fprintf`
diagnostics, which only fire on the corrupted path (i.e. only when the bug is about to manifest anyway).

## References
- [[reference_openswiftui_headless_uaf_repro]] — the deterministic repro + full run-by-run log of this
  investigation (chronological; this doc is the organized synthesis).
- [[reference_openswiftui_immortal_fix]] — earlier (superseded) per-site guard attempts; kept for history,
  not to be repeated (each guard just relocated the crash — this investigation explains why: those were
  symptom sites, not the corrupting write).
- [[feedback_compute_goal_and_working_rules]] — the binding process rules this investigation followed.
