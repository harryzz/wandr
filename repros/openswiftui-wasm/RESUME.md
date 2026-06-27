# ✅✅ CRASH ROOT CAUSE FOUND + FIXED (2026-06-27 pm): unbounded existential-compare recursion → shadow-stack overflow into Swift metadata pool

**The post-freeze `uninitialized element` crash was UNBOUNDED RECURSION in Compute's existential comparison.**
`compare_existential_values` (LayoutDescriptor.cpp ~595) fetched the layout of the EXISTENTIAL CONTAINER
type (`fetch(reinterpret_cast<const swift::metadata&>(type),...)`) and walked it over the PROJECTED payload.
That layout's first entry is itself an `Existential`, so `compare()` re-entered `compare_existential_values`
on the SAME type without bound → the wasm shadow stack (no guard page) overflowed INTO the Swift metadata
pool → the shared `any ViewList` existential metadata's VWT slot (mem[meta-4]) was zeroed → the value-witness
`destroy` in the `RuleContext.value` setter dispatched through a NULL funcref → `wasm trap: uninitialized element`.

**THE FIX (LayoutDescriptor.cpp compare_existential_values):** compare the projected payload against its
DYNAMIC type's layout/size, not the existential container's:
```c
  ValueLayout wrapped_layout = fetch(*lhs_dynamic_type, options, 0);          // was: (swift::metadata&)type
  return compare(layout, lhs_value, rhs_value, lhs_dynamic_type->vw_size(), options);  // was: type.vw_size()
```
This bounds the recursion to the real nesting depth (WrappedList.base unwraps to EmptyViewList/array). PROVEN
by a write-tripwire on the victim metadata's VWT slot (latched in value_set_internal): goes SILENT with the fix;
board now plays (`MOVE 0 tiles=3`, `MOVE 1 tiles=4`) where it crashed at move 0 before.

METHOD: a write-tripwire (`*((void*const*)&metadata - 1) == nullptr → __builtin_trap`) anchored on
value_set_internal's `metadata` param + `-D max-backtrace` named the exact step (`compare_values`) and then the
exact recursion site, after 4 agent theories (CoW destructive-compare, region relocation, immortal-Subgraph,
InlineHeap) all FAILED their build-test. Lesson: when theories keep failing, instrument to catch the writer.

Also applied (device-verified-correct per [[reference-openswiftui-immortal-fix]], but NOT this crash's cause):
immortal Subgraph CF storage — gate `CFRelease` of IAGSubgraphStorage under `!__wasi__` (Subgraph.cpp clear_object,
IAGSubgraph.cpp IAGSubgraphSetCurrent), since objc_bridge(id) is empty on wasm so Swift can't ARC-manage refs.

## 🔲 NEXT (still RED): "reading from invalid source attribute" after move 2
`DynamicLayoutViewChildGeometry.updateValue` (DynamicLayoutView.swift:108) reads `childGeometries.count` →
`LayoutView` (LayoutView.swift:189) reads `layoutComputer` (an attr in an INVALIDATED subgraph) → add_input
precondition (Graph.cpp:679). This is the DynamicLayoutViewChildGeometry/"accessing invalidated subgraph" family
the immortal memo lists as band-aided. The CF immortal-storage fix keeps the STORAGE alive but the graph still
invalidates the NODE, so it doesn't cover this. Needs the invalidation-lifecycle fix (or the guard the prior
session used). TODO.

### Triangulation (2026-06-27 pm) — move-2 root is Compute page-recycle, NOT the OpenSwiftUI reconciliation
Diffed our OpenSwiftUI vs upstream OpenSwiftUIProject/main: the reconciliation LOGIC is upstream-faithful
(DynamicContainer/DynamicView/ViewListContent = upstream `Status: Complete`; DynamicLayoutView identical
except our transition-capability gate). The ONLY divergences we carry are BAND-AIDS for ONE family —
"an invalidated subgraph referenced while its page is recycled": DynamicContainer `if subgraph.isValid`
guard on `subgraph.index=`; DynamicView `item.isValid` guard; ViewListContent wasm `isIdentical` workaround.
Apple/upstream needs none (Apple AG keeps an invalidated subgraph's storage alive PAST the readers); our
wasm Compute recycles its PAGE eagerly (zone_id churn 205→141 under a live weak indirect = the move-2 fault).
Band-aiding the un-guarded point (Compute add_input weak-indirect read) just MOVES the symptom to the next
reader — guards can't fix it. REJECTED approaches: (a) add_input weak-tolerance (moved symptom LayoutView→
RendererLeafView); (b) main-handler graft (Part A drain in Graph::with_main_handler + Part B
WandrRendererHost.renderOnce withMainThreadHandler wrap) — REGRESSED to a move-0 crash because it grafts
Apple's ASYNC-render main-handler onto our SYNC render path. **REAL FIX (in progress) = Table page-quarantine**
(prior session's Table.h+Table.cpp; the immortal memo says it lets the band-aids be deleted): epoch-tag pages
freed by a mid-update subgraph invalidation; alloc_page may only reuse pages from a strictly-earlier epoch;
bump epoch at the outermost-update boundary → reuse always lands past the readers, render-path-agnostic,
self-reclaiming (no leak). KEY LESSON (user, 30 yrs): OpenSwiftUICore is a REIMPLEMENTATION — "works on Apple"
only proves the `Status: Complete` files; WIP/Blocked paths (and our band-aids) can diverge, so the fix may be
in our Compute OR our OpenSwiftUI band-aids, just never in code that faithfully mirrors Apple.

### ✅ move-2 FIXED + committed; move-4 = transition/animation (our refactor)
move-2 fix LANDED (committed: OpenSwiftUI ae554150 — WandrRendererHost.renderOnce/redraw wrapped in
`viewGraph.graph.withoutSubgraphInvalidation`; Compute 5b40c3c — reverted the per-update dtor flush). The
side-effect-free deferral (toggles only _deferring_subgraph_invalidation, NOT main-thread dispatch like
withMainThreadHandler which regressed move-0) holds invalidated-subgraph teardown across the whole render →
no mid-render page free under live readers. Board now plays MOVE 0-3 (merges work, score=8).
move-3→4 follow-on (UpdateStack::update updating an invalidated-DEFERRED subgraph) was RULED OUT — a dispatch-site
`is_valid()` guard did NOT clear move-4 (failing node's subgraph is valid). **move-4 root = the tile
ANIMATION/TRANSITION path**: failing rule = `ForEachChild<…ModifiedContent<…TileView,_FrameLayout>,_PositionLayout>,
_TraitWritingModifier<TransitionTraitKey>>,_AnimationModifier<CGPoint>>>` dispatches an OOB funcref (node valid,
type_id valid). Seat = OUR "rendering-capability" refactor: WandrApp sets `supportsViewTransitions: false`, the
DynamicLayoutView gate (`if let transition, …supportsViewTransitions`) renders the element DIRECTLY but the content
keeps the Transition/Animation modifiers. Our divergences vs upstream: RendererConfiguration.swift (−87),
ForEach.swift (−10), DynamicLayoutView gate. Prior session ([[reference-openswiftui-immortal-fix]]) ran this demo
with **transitions ON** (`supportsViewTransitions: true` + a DynamicContainer index fix, already present at :249).
So move-4 = our incomplete/regressed transition-animation handling, NOT Compute.

## ◀ REMAINING TASK (the ONLY thing between here and a fully-playable render) — 2026-06-27 consolidation
**Everything deep is FIXED + COMMITTED. Engine + game logic + stability = SOLVED.** Proven: with
`supportsViewTransitions:true` (WandrApp.swift:54) the demo plays ALL 60 moves, score=4264, exit 0, ZERO
crashes. The remaining bug is ONLY the **animated-tile-content rendering** — our `[WIP] ContentTransition`
machinery (DynamicLayoutView.swift: `MakeTransition`/`ViewListTransition`/`T.makeView`, lines ~121-163,246-259).
It has TWO manifestations, ONE root:
- **supportsViewTransitions=false (current committed baseline):** tiles render correctly (text+positions,
  `rendered=["2@363,166",…]`) for moves 0-3, then the animated `ForEachChild` update OOBs at move 4:
  `wasm trap: undefined element: out of bounds table access`, bounded 19-frame recursion (NOT a cycle),
  bottom = `Rule.swift:29`/`UpdateStack.cpp:258` rule-body dispatch for
  `type=ForEachChild<Array<IndexedTile<IdentifiedTile>>, Int, ModifiedContent<…TileView,_FrameLayout>,
  _PositionLayout>, _TraitWritingModifier<TransitionTraitKey>>, _AnimationModifier<CGPoint>>>`. The failing
  node has a VALID subgraph and VALID type_id (~75) — so it's a corrupt funcref in the content's update,
  NOT an invalidated subgraph (a dispatch-site `is_valid()` guard did NOT help) and NOT a corrupt type_id.
- **supportsViewTransitions=true:** no crash (all 60 moves) BUT the transition path drops the tile values
  → `CREATEBLOCK val=nil`, no text drawn, `rendered=[]` (genuinely blank; bodies ARE evaluated [61 BODY-EVAL]
  and blocks ARE created [1493], but with nil values). So the transition `T.makeView`/`ViewListTransition`
  doesn't propagate the element's value/geometry to the display list.

**TWO ways to finish (either gives renders + 60 moves):**
  A. **Complete the transition value-propagation** (with transitions ON): fix `MakeTransition.visit` →
     `T.makeView(view:inputs:body:)` / `ViewListTransition` so the element body's value/geometry reach the
     display list (currently blocks come through val=nil). This is the prior session's path (transitions ON
     worked there). Start: DynamicLayoutView.swift:130-153 + the Transition.makeView impl + ViewListTransition.
  B. **Fix the ForEachChild OOB in the direct path** (transitions OFF, keeps the working render): coredump the
     OOB call_indirect at the animated `ForEachChild` update (`-D coredump -D debug-info=y`; recover the OOB
     funcref index + the node's `self`/content witness). The corrupt dispatch is reached only via the direct
     render (`outputs = body(elementInputs)`, DynamicLayoutView.swift:258), NOT the transition path — so it's
     in how the direct path constructs/updates the `_AnimationModifier<CGPoint>` content.
  Recommend A (matches the prior working approach; transitions ON already proves the engine is solid).

**Repro:** `cd repros/openswiftui-wasm/probe && bash build-wasi.sh && wasmtime run --env
SWIFT_DETERMINISTIC_HASHING=1 -W max-wasm-stack=8388608 .build/wasm32-unknown-wasip1/debug/probe.wasm`.
Flip WandrApp.swift:54 supportsViewTransitions true/false to switch manifestations. Method that works here:
instrument/coredump to catch the exact writer (NOT theorize — relocation/main-handler theories both failed
their build-test this session; the write-tripwire + the transitions-ON A/B are what actually localized things).

### PATH A EXPLORED (2026-06-27 pm) — partial, reverted; key learnings
Tried completing the transition render (transitions ON). Findings:
- The blank-render root = `ApplyTransitionModifier._makeView` (Transition.swift:151) is an UPSTREAM STUB returning
  `.init()` (drops the wrapped content; PlaceholderContentView never resolved → makeElt never called). Filling it
  with the standard passthrough `body(_Graph(), inputs)` MAKES TRANSITIONS RENDER (tiles draw via the transition
  path, MK-BODY fires). **But NOT keepable as-is for Path B:** the tile's `.transition(…)` modifier routes through
  ApplyTransitionModifier._makeView in the DIRECT path too, so the passthrough changes direct-render behavior →
  regressed. (Reverted; re-apply only alongside transitions-ON for a future animated renderer.)
- With transitions ON + the _makeView fix, the next crash is a CHAIN of transition-machinery bugs: AnyTransition.swift:20
  / CombiningTransition OOB-MEMORY evaluating the eleev tile's composed `AsymmetricTransition<Combining<Opacity,Move>,…>`
  (AnyTransition+TileGenerator.swift:19). Transitions are genuinely WIP/unimplemented upstream → finishing them is a
  multi-bug effort for an effect a STATIC snapshot renderer can't even show.
- A probe-only debug print (TileBoardView.swift:70 `String(describing: block?.value)`) had its OWN value-witness OOB
  (red herring; the eleev demo's debug line, not core).
- **The move-2-4 crash is BUILD-LAYOUT-SENSITIVE** (recursive ForEach OOB-memory at move 2 ⟷ ForEachChild OOB-table at
  move 4, varies per rebuild) = the signature of METADATA CORRUPTION (same family as the core existential-recursion
  crash). So the remaining direct-path bug is a deep, build-sensitive metadata-corruption issue in the tile content
  rendering (nested ForEach / animated `_AnimationModifier<CGPoint>` content) — needs the write-tripwire/coredump
  method in a fresh focused session (a build-sensitive bug can't be pinned by reading/theorizing).
RECOMMENDATION STANDS: static renderer doesn't need transitions; finish via Path B = pin the build-sensitive
tile-content metadata-corruption crash with a write-tripwire on the corrupted metadata/funcref, like the core crash.

---

# ✅ ROOT CAUSE FOUND + FIXED (2026-06-27): existential compare_values copy-paste bug in Compute

**The freeze was a one-line copy-paste bug in Compute's value comparison**, NOT in OpenSwiftUI.

`Compute/Sources/ComputeCxx/Comparison/LayoutDescriptor.cpp`, `compare_existential_values` (~line 588):
```cpp
unsigned char *lhs_value = (unsigned char *)type.project_value((void *)lhs);
unsigned char *rhs_value = (unsigned char *)type.project_value((void *)lhs);  // BUG: lhs → must be rhs
```
`rhs_value` was projected from `lhs`, so **every existential (`any ViewList`, `any View`, …) compared
against itself → always "equal/unchanged."** The graph therefore never propagated changes through the
existential-typed attributes the whole ViewList/DisplayList pipeline runs on → dynamic ForEach never
re-materialized → board frozen at the 2 initial tiles. **Fix: `lhs` → `rhs`.** With the fix the probe's
board grows (`items` 2 → 4 …).

**SECOND issue, now unmasked** (prior session also found it): fixing the compare surfaces a
`wasm trap: uninitialized element` in `ForEachList.Init.updateValue` → `RuleContext.value` setter
(the StatefulRule update path). Prior (wiped) session attributed this to a **double-destroy** in
`Compute/Sources/ComputeCxx/Subgraph/Subgraph.cpp` (indirect-node teardown — two passes: Loop1
`remove_node`/`remove_indirect_node` @233-250, Loop2 `node->destroy` @253-279) + a `Data/Table.{h,cpp}`
quarantine. STILL TO FIX. The crash is now reachable only because growth works.

**Why it took so long (process lesson — see `[[feedback_read_source_first]]`):** the symptom is "change
detection fails" → the governing primitive is `compare_values` in Compute. Instead of reading that
primitive, ~16 rebuild-and-trace cycles were spent in OpenSwiftUI (ForEach/DynamicContainer/DynamicView
isValid guards, render drive) chasing symptoms one layer up. The fix was only found after going DOWN
to the Compute comparison code. **Rule: when change-detection misbehaves, test `compare_values` in
isolation FIRST.**

---

# 🐞 OPEN REGRESSION: dynamic-ForEach render FREEZE on the clean-shim stack (2026-06-26)

**Symptom:** the eleev/2048 demo renders a **static board** — the visual (wandr-host desktop) and
the stdout probe both show only the **2 initial tiles, frozen at fixed positions**, while the game
model advances normally. The board never updates as tiles spawn/slide/merge.

**This is a NEW regression introduced TODAY by `0f4f20bf` (OpenSwiftUI "consumer-side integration
for Compute's WasiClosureShim").** It is the ONLY OpenSwiftUI commit since the demo last animated
("2 days ago" per user). Everything below `0f4f20bf` is dated 2026-06-19 and was working.

## Stack / how to repro
- Clean stack (persistent): `~/wandr/tests/OpenSwiftUIProject/{Compute,OpenAttributeGraph,OpenSwiftUI}`.
  - `Compute` HEAD = the `WasiClosureShim` (`1a3c4a3`); `OpenSwiftUI` HEAD = `0f4f20bf`.
- **Probe (fast, stdout, the diagnosis vehicle):** `repros/openswiftui-wasm/probe` →
  `bash build-wasi.sh` then `wasmtime run --env SWIFT_DETERMINISTIC_HASHING=1 -W max-wasm-stack=8388608
  .build/wasm32-unknown-wasip1/debug/probe.wasm`. Drives the EXACT demo render path
  (`wandrApplyChange`→`wandrRender`→`wandrRedraw`), 60 moves, prints the rendered board.
- **Visual demo:** `repros/swift-canvas-spike/build-openswiftui-demo.sh` → run on x86_64 wandr-host:
  `env -u WAYLAND_DISPLAY DISPLAY=:0 WINIT_UNIX_BACKEND=x11 WANDR_DESKTOP_SIZE=500x1000 setsid
  runtime/wandr-host/target/x86_64-unknown-linux-gnu/release/wasm-android-host
  repros/swift-canvas-spike/openswiftui-demo.component.wasm` — **MUST run with Bash
  `dangerouslyDisableSandbox:true`** (the sandbox SIGKILLs the GUI/X11 → exit 144). Wayland crashes
  on WSLg; force X11.

## Diagnosis (probe-proven — ruled OUT and CONFIRMED, layer by layer)
- ❌ NOT the host present/redraw (earlier Slint/dioxus + 2-days-ago 2048 animated fine).
- ❌ NOT the present rate / WSLg flooding (same uncapped auto-play worked 2 days ago).
- ❌ NOT the `render(style:) is unimplemented` warning (benign — handles `.color` fills at
  ShapeStyleRendering.swift:186-192; line 203 is a blanket shape-EFFECTS FIXME, fires on every shape).
- ❌ NOT the `0f4f20bf` `isValid` guards — `Subgraph::is_valid()` = `_invalidation_state == None`,
  correctly TRUE for live items.
- ✅ `ContentView.body` re-evals every move (`BODY-EVAL` 61×, reads `tiles=2→11`).
- ✅ `TileBoardView` re-renders with the **new matrix** (`TBV-BODY matrix=2→11` with live tile values).
- ✅ The tiles `ForEach(matrix.flatten(), id: \.tile.id)` rule `update(view:)` gets the **new data**
  (`FES-UPDATE n=5,6,7,8,9,10,11` all observed; `const=false` everywhere — NOT the constant-data path).
- ✅ Items are **created and kept** (`FES-NEW=22`, `FES-ERASE=2`).
- ❌ **The render walks a STALE `ForEachState`.** `FES-NEW` shows a state being updated with
  `data=2` (the initial view) at the END of the run, while `update(view:)` elsewhere gets `n=11`.
- **Render gate:** `ForEachChild.updateValue` (ForEach.swift:1486) emits a view only if
  `state.items[id]` exists AND `item.seed == state.seed`. Only the stale state's 2 items pass.

## ROOT HYPOTHESIS (where a fresh session should start)
**The `ForEachState`/`Info` attribute IDENTITY is splitting under the clean-shim foreign-ref
`Subgraph`.** `ForEachChild`'s `@Attribute var info` resolves to a DIFFERENT `ForEachState` instance
(stale, `data=2`) than the one `update(view:)` mutates with the live data. The render therefore
walks 2 frozen tiles. Suspect surface = exactly what `0f4f20bf` changed around Subgraph identity:
`ScrapeableContent._scrapeID`/`rawIdentity`, `Subgraph.isIdentical`, `ViewListContent` child wiring.

### Three concrete next experiments (cheapest discriminator first)
1. **Bisect:** temporarily revert ONLY the `0f4f20bf` `ScrapeableContent._scrapeID`/`rawIdentity`
   change (Data/Util/ScrapeableContent.swift) and re-run the probe. If tiles animate → culprit found.
2. Trace `ForEachChild.info` attribute identity (the `Info` AGAttribute id) vs the id of the state
   `update(view:)` mutates — confirm they diverge.
3. Check `Subgraph.rawIdentity`/`isIdentical` stability for the ForEach child subgraph across moves
   (is the foreign-ref identity unstable, splitting the StatefulRule state?).

## Instrumentation STILL IN PLACE (remove or reuse)
- `OpenSwiftUI/.../ForEach.swift`: `print()` traces — `FES-UPDATE` (~line 344), `FES-NEW`
  (`item(at:)` else), `FES-ERASE` (`eraseItem`), `FES-WALK` (in `estimatedCount` — **never fires**,
  delete it).
- `probe/Sources/Probe/ProbeApp.swift`: `BODY-EVAL` in `ContentView.body`; `PrintSink.drawText`
  records `"value@x,y"`; `wandrRedraw()` added to the 60-move loop (matches the demo paint path).
- `probe/Sources/Probe/eleev/TileBoardView.swift`: `TBV-BODY` print at top of `body`.
- `swift-canvas-spike`: demo build re-pointed to the persistent stack; `ComputeStubs` target added
  (print_cycle stub, needs `include/` dir); `-Xcc -include wasi_compat.h` added to the build script
  (the `uint` gap). The bounded-5000-move auto-play in `OpenSwiftUIDemo/main.swift` is intact.

---

# ✅ ALL THREE BUGS FIXED + DEVICE-VERIFIED (2026-06-25)

**The aarch64 device `0.42` crash was NOT a "wasmtime Cranelift miscompile."** It was the
cross-module foreign-reference **over-release** all along (a float `0.42` landing where a freed
`Subgraph` storage pointer was reused). Proven by: making the CF storage **immortal** (Swift
foreign-ref no-op retain/release) eliminates the crash on BOTH x86 (desktop) AND aarch64 (device,
Pixel 2 XL cross-AOT), under the full auto-play + `interpolatingSpring` animation + transitions.

## Root cause (unified for B1 over-release + B2 animation crash)
Off-Apple there is no `objc_bridge` to unify Swift ARC with the CF refcount. The fork imported
`Subgraph` as a foreign-reference type with `retain:IAGSubgraphRetainRef`/`release:...`, but the
cross-module retain/release is **asymmetric** (e.g. `_ViewList_Subgraph.deinit` / `ItemInfo`
array-destroy releases a storage the live graph node still references) → over-release → double-free
/ UAF. The animation just amplifies the array churn (same root).

## THE FIX (clean, no guards)
1. **Immortal storage** — `IAGSubgraphRetainRef`/`ReleaseRef` are no-ops on wasm (a legitimate
   Swift foreign-reference *immortal* mode). The small CF wrapper leaks (bounded); the `IAG::Subgraph`
   node it points to is still freed normally by the graph. Eliminates over-release/double-free/UAF
   for **every** Subgraph reference at once.
2. **B3 transitions** — set `supportsViewTransitions: true` + fixed the upstream constant-index bug
   at `DynamicContainer.swift` line ~440 (`displayMap[validCount]` → `displayMap[validCount + index]`;
   only reachable when `removedCount != 0`, i.e. transitions on).
3. Real roots still in place: `Data/Table.cpp` zone-zeroing (wasi mmap), Subgraph/Graph member inits.

## ALL band-aids REMOVED (were dead code once immortal):
`from_cf` liveness guard, all 11 softened "accessing invalidated subgraph" preconditions,
`DynamicLayoutViewChildGeometry` offscreen/unconditional-set (reverted to upstream), the
`DynamicContainer.swift:453` isValid guard. All debug instrumentation removed.

## Verified
- Desktop x86 JIT: 3/3 × 90s survive, animation + transitions ON, 0 faults, 0 dead-hits.
- Device Pixel 2 XL aarch64 cross-AOT: deployed, auto-plays (move/3 frames, thousands of springs),
  STABLE, NO `0.42`. The crash a prior session called an unfixable wasmtime miscompile is GONE.
- Known tradeoff: immortal = small bounded leak (CF storage wrappers only). Future: real ref-counted
  free if the leak ever matters.

---
(historical notes below)

# OpenSwiftUI on wasm — resume point (updated 2026-06-24)

## 🟢 DEVICE ROOT CAUSE FOUND — it's a Wasmtime aarch64 Cranelift MISCOMPILE, not our code (2026-06-24)
The device `0.42` "UAF" (SIGSEGV, `x4=0x3ed70e9c` over a `Subgraph*`, in the geometry/display-list walk
`function[3559]`/`[3564]`/`[3598]`) is **NOT a use-after-free in OpenSwiftUI/Compute and NOT a foreign-ref
ARC bug.** It is an **aarch64-only Wasmtime Cranelift code-generation miscompile.** Proven by a clean A/B
of the *identical* demo (real canvas renderer + interpolatingSpring animation + auto-moves):

| Target | Renderer | Result |
|---|---|---|
| x86 **JIT** (desktop host) | real canvas | ✅ survives 45 s, no fault |
| x86 **AOT** (`wasmtime compile` probe) | PrintSink | ✅ survives 60 moves, exit 0 |
| **aarch64 AOT** (device) | real canvas | ❌ `0.42` SIGSEGV on manual play |

So the only variable that flips survive→crash is the **target arch (x86 → aarch64)**; JIT-vs-AOT and our
code are held identical. The foreign-ref machinery was verified CORRECT (CF `CFRuntimeBase` storage,
`CFRetain`/`CFRelease`, right `RETURNS_RETAINED`/`UNRETAINED` annotations — Swift Forums + cxx-interop docs).

RULED OUT:
- **Not the foreign-ref/Subgraph lifetime** (machinery correct; x86 runs the identical path clean).
- **Not CVE-2026-34971** (that aarch64 Cranelift load miscompile needs memory64 + was fixed ≤43.0.1; we're
  wasm32 on 45→46). But it proves the *class* of aarch64 Cranelift miscompiles is real & recent.
- **Not a Wasmtime version regression**: persists on **45.0.0 AND 46.0.0** (host bumped to 46 — clean, kept;
  one API fix: `Component::exports` now yields `ComponentExtern`, fixed in `app_loader.rs`).
- **opt-level=none does NOT fix it** (2026-06-24): re-AOT'd the device at `cranelift_opt_level(None)` →
  the `0.42` didn't recur but a DIFFERENT aarch64 miscompile appeared (`0x690` SEGV_MAPERR, crash-loop in
  the launcher's Compose code). So the aarch64 backend miscompiles at BOTH opt levels, different code. Not a
  usable workaround → REVERTED (device restored to wasmtime-46 speed, stable).

CONSEQUENCES — everything downstream was a SYMPTOM of garbage pointers, not real bugs:
- The load-bearing `isValid` guards (ForEach.eraseItem, ViewList.remove, DynamicView×2, DynamicContainer)
  STAY — they're the only on-device mitigation until the aarch64 codegen is fixed. (Stripping them →
  immediate crash, confirmed.)
- The merge-ghost and the "transitions blank off-Apple" are very likely the SAME garbage-pointer corruption,
  not separate OpenSwiftUI gaps. Do not chase them as framework bugs until the codegen root is fixed.
- Shippable device config = **animation OFF + guards** (mostly stable; the spring is the strongest trigger).

NEXT: (1) minimize a repro + file upstream at github.com/bytecodealliance/wasmtime (the A/B + the `0.42`
fault is the evidence); (2) try a Wasmtime git-main / nightly (a post-46 aarch64 Cranelift fix may exist);
(3) commit the wasmtime-46 bump. The desktop integration remains the solid, shipped deliverable.

---
## (superseded) earlier guess — 2 deep off-Apple roots
Desktop (x86 JIT) is fully green. On the **Pixel 2 XL (aarch64 cross-AOT)** the demo renders + redraws
+ is launch-stable, but TWO real off-Apple/wasm-platform gaps remain — both BELOW the OpenSwiftUI/Compute
source we control, established by direct device experiments this session:

1. **aarch64 `Subgraph`-lifetime UAF** (the crash + the merge-ghost share this root).
   - Real play with the position `.animation(.interpolatingSpring…)` ON → SIGSEGV, `x4=0x3ed70e9c` (the
     interpolated 0.42 CGFloat) over a `Subgraph*`, `ui.cwasm function[3598]+16` (a display-list/geometry
     traversal walking a stale subgraph). **Auto-moves don't trip it; varied real moves do.**
   - The foreign-ref `Subgraph` ARC (CFRetain/CFRelease) keeps storage alive on **x86 JIT** (probe survives
     3/3 across heap layouts — robust, not luck) but **NOT under aarch64 cross-AOT** → storage freed/reused
     (becomes the 0.42 ViewGeometry), later traversal faults. Same wasm bytecode, different wasmtime
     compile → a **wasmtime aarch64-AOT codegen** (or CF-refcount-on-aarch64) issue. NEXT decisive probe:
     cross-AOT the SAME wasm for x86 and run locally — if it crashes too → AOT-codegen bug, not aarch64.
   - The `isValid` guards (ForEach.eraseItem, ViewList.remove, DynamicView lastItem+reuse, DynamicContainer
     index) are **LOAD-BEARING on aarch64** — reverting them to upstream → crash returns on real play. They
     are NOT redundant (the foreign-ref doesn't fully protect aarch64). Keep them until the root is fixed.
   - **Merge-ghost** (old value drawn under merged tile, e.g. "8" under "16") = SAME root: a removed child
     whose storage the aarch64 ARC freed leaves a stale parent `_children` entry that keeps rendering.
     Traced ALL removal paths — `IAGSubgraphRemoveChild` (`if(child->subgraph)` no-ops on a nulled child),
     `Subgraph::remove_child`, `invalidate_and_delete_` (does `for parent: parent->remove_child(*this)`) —
     they all clean up correctly on a VALID lifecycle, so the ghost only appears once the aarch64 ARC has
     already corrupted the lifecycle. Not cleanly fixable above the lifetime root.
   - **MITIGATION (shipped):** `TileBoardView.swift` position `.animation` is DISABLED → no crash, tiles
     snap, board redraws. Cosmetic ghost may still appear intermittently on merges.

2. **Off-Apple transition rendering is a genuine gap** (separate from #1).
   - `_RenderingCapabilities.supportsViewTransitions` is forced **false** in `WandrApp.swift`. Flipping it
     **true** → the board renders as a **solid grey rectangle (no grid, no tiles)**: `Transition.makeView`
     (pushes the element as a `PlaceholderContentView` modifier-body for the transition's `Body`) does NOT
     produce the element output off-Apple. Confirmed NOT caused by our guards (stays blank with all guards
     reverted) and NOT the crash (no crash, just blank). `Animation/Transition/*` is full of
     `_openSwiftUIUnimplementedFailure()` stubs. Implementing this is the principled fix that would let
     transitions render → proper insert/remove → no ghost, *if* #1 is also solved.

   **Redraw was a real bug, now FIXED** (do not regress): off-Apple value comparison must `memcmp` —
   `AttributeType.h::compare_values` + `compare_values_partial` and `IAGComparison.cpp::IAGCompareValues`
   each have `#if defined(__wasi__) … memcmp(…vw_size())…`. Without it, @State changes don't re-render.

   Principle (user, 2026-06-24): do NOT patch core OpenSwiftUI or hack one app to "work" — fix the
   non-Apple path so all apps work. The guards are a tolerated stopgap ONLY because the aarch64 root is
   below us; the two roots above are the real "make OpenSwiftUI work off-Apple" project.

---
# OpenSwiftUI on wasm — resume point (updated 2026-06-23, ✅ swiftui-2048 PLAYABLE on desktop)

## ✅✅✅ INTEGRATED onto UPSTREAM Compute (2026-06-23 latest) — foreign-ref the blessed way; 3/3 survive
REPLACED the stale AG*-named fork with the **current upstream IAG\*** trees and re-did the fix with the
upstream foreign-reference machinery (`SWIFT_SHARED_REFERENCE`), per user direction. **Desktop: builds
green + 60-move probe SURVIVES 3/3 (1 deterministic + 2 non-deterministic heaps), exit 0, board renders.**
The UAF is fixed by the upstream-blessed foreign-ref — NOT the old hand-rolled item-retain band-aids.

WHAT CHANGED (the integration):
- **Archived** old `/tmp/Compute` + `/tmp/oag-fork` → `/tmp/graph-backup-20260623/` (AG* names; stale since
  jcmosc commit `1dad53817` "Rename AG namespace to IAG", **2026-06-17 12:04 UTC** — our fork branched
  just before it). Cloned FRESH: **`jcmosc/Compute@fix-compatibility-tests`** (has `Utilities/SwiftBridging.h`
  + `IAG_SWIFT_SHARED_REFERENCE`) → `/tmp/Compute`; **`OpenSwiftUIProject/OpenAttributeGraph@main`** (already
  post-PR-229 = IAG* adapter typealiases) → `/tmp/oag-fork`. Same paths → relative dep-wiring intact.
- **Key naming fact:** the C layer is `IAG*` (IAGSubgraph.h, IAG::, IAGGraphReadCachedAttributeC…) but the
  Swift-visible names stay CLEAN (`Subgraph`, `AnyAttribute`, `Graph`, `CachedValueOptions`) via
  `IAG_SWIFT_NAME`; OAG `OAG*` typealiases now point at `IAG*`. **OpenSwiftUI references Compute ZERO times**
  (only comments) — fully vendor-neutral via OpenAttributeGraphShims.
- **Re-ported the wasm engine work** (the 3 wasm commits, 27 files) onto IAG* via an `AG→IAG` token
  translation of the committed diff + `git apply --reject`; 7 rejects hand-fixed (the `*C` swiftcall-mislower
  variants `internAttributeType`/`_cachedValue`/`setOutputValue`/`IAGRetainClosureC`/`apply_c`/Context
  callback against upstream's REFACTORED code; `static_assert(sizeof…)` wasm guards). The
  `Submodules/swift-runtime-headers` (vendored swift/Runtime/*) had to be copied from the backup (the
  depth-1 clone didn't populate the submodule; Package.swift already had the `-isystem` flags).
- **`Subgraph` → foreign-reference type** on wasm: `IAGSubgraph.h` typedef `IAG_SWIFT_SHARED_REFERENCE(
  IAGSubgraphRetainRef, IAGSubgraphReleaseRef)` + `IAG_SWIFT_RETURNS_RETAINED/UNRETAINED` (added to IAGBase.h)
  on Create/GetCurrent/GetChild/…; retain/release wrap CFRetain/CFRelease (IAGSubgraph.cpp). **NOTE: works
  even though the Compute Swift target has NO `.interoperabilityMode(.Cxx)` — `swift_attr("import_reference")`
  is processed by the ClangImporter and ARC manages the storage. The earlier "needs cxx interop" fear was
  WRONG; enabling cxx interop globally is in fact IMPOSSIBLE on wasm (cycles `SwiftWASILibc -> std_inttypes_h`).**
- **Cross-module member facade** (`Compute/Graph/Subgraph.swift`, `#if arch(wasm32)`): a foreign-ref class's
  importer-synthesized members (self: instance members, statics, nested Flags) DON'T cross `@_exported import`
  to consumers — so `IAGSubgraph.h` drops the `self:` swift_name on wasm (`IAG_SUBGRAPH_SELF_NAME` macro =
  empty) → each imports as a refined free func `__IAGSubgraph…`, and the facade re-declares ALL of them as
  Swift EXTENSION members (which DO cross). Includes `apply_c`/`IAGSubgraphApplyC` + `forEach` C-routing,
  free-func inits `Subgraph(graph:)`/`Subgraph(graph:attribute:)`, and `IAGAttribute.h` Flags swift_name
  gated off on wasm.
- **Last-mile class-identity (5 sites, 2 files):** foreign-ref types DON'T support `===`/`ObjectIdentifier`
  (and the class-ness doesn't cross module anyway). Facade provides `var rawIdentity: UInt` (storage-pointer
  bits via `unsafeBitCast(self, to: UnsafeRawPointer.self)`) + `func isIdentical(to:)`. Patched (wasm-gated,
  whole-statement to keep braces balanced — splitting `if/guard … {` across `#if` is a syntax error):
  `ScrapeableContent.swift` (typealias `_ScrapeSubgraphID` = UInt + `_scrapeID()`; `Set<UInt>`),
  `ViewListContent.swift:233` (`!==` → `isIdentical`). (`ViewList.swift:2430` `lhs.subgraph === rhs` is a
  native `_ViewList_Subgraph` class — left alone.) One API shim: `Graph.archiveJSON(name:)` static added to
  the OAG adapter (OpenSwiftUI calls it statically; Compute has only the instance variant).
- REPRO: `cd repros/openswiftui-wasm/probe` + the build cmd in §"Build/run the probe" (now resolves to the
  IAG* trees) → `wasmtime run --env SWIFT_DETERMINISTIC_HASHING=1 -W max-wasm-stack=8388608 .build/.../probe.wasm`.
- ✅ WORK SYNCED INTO PATCHES (the live /tmp trees + these 3 patches both hold it):
  `compute-wasm.patch` (IAG* Compute, 32 files), `openswiftui-phase1-wip.patch` (OpenSwiftUI wasm, 9 files),
  `oag-shims-wasm.patch` (NEW — OAG `Graph.archiveJSON` static shim, 1 file). RECREATE the trees from scratch:
  ```
  git clone --depth 1 -b fix-compatibility-tests https://github.com/jcmosc/Compute /tmp/Compute
  git clone --depth 1 -b main https://github.com/OpenSwiftUIProject/OpenAttributeGraph /tmp/oag-fork
  ln -sf /tmp/oag-fork /tmp/OpenAttributeGraph   # OpenSwiftUI deps resolve ../OpenAttributeGraph
  # populate the vendored swift/Runtime headers the patch EXCLUDES (depth-1 didn't fetch the submodule):
  cp -r /tmp/graph-backup-20260623/Compute/Submodules/swift-runtime-headers /tmp/Compute/Submodules/ \
    && rm -rf /tmp/Compute/Submodules/swift-runtime-headers/.git   # (or: git submodule update --init)
  cd /tmp/Compute  && git apply <repo>/repros/openswiftui-wasm/compute-wasm.patch
  cd /tmp/oag-fork && git apply <repo>/repros/openswiftui-wasm/oag-shims-wasm.patch
  cd /tmp/OpenSwiftUI && git apply <repo>/repros/openswiftui-wasm/openswiftui-phase1-wip.patch   # (fork already on wasm32-wasip1)
  ```
  (The old `oag-fork.patch` is the STALE AG*-era OAG-default-backend diff — superseded; not used by this Compute-backend integration.)

### ✅ DEVICE (Pixel 2 XL, cross-AOT aarch64) — renders, REDRAWS, no ghost, playable
Cross-AOT'd the integrated stack to aarch64 + deployed (`wandr.swiftui.demo`). The 2048 board RENDERS and
REDRAWS after moves (user-confirmed on-device), tiles clean (no merge ghost). Three device-session fixes
on top of the integration (all in the patches):
- **REDRAW fix (the re-port gap I missed):** the wasm `memcmp` comparison fallback was only in the
  COMMITTED diff's `LayoutDescriptor.cpp`, NOT in `AttributeType.h` (`compare_values` +
  `compare_values_partial`) or `IAGComparison.cpp` (`IAGCompareValues`) — those were in the working-tree
  part. Without them `LayoutDescriptor::compare` returns "equal" for CHANGED non-trivial views on wasm32 →
  no re-eval → board never repaints after a move. Re-added all 3 (compute-wasm.patch). `compare_values_partial`
  also guards the pow-SIGILL (Equatable dispatched with a wrong pointer → OOB).
- **MERGE-GHOST fix (`ForEach.eraseItem`, openswiftui-phase1-wip.patch):** the `isValid` guard (added to stop
  the UAF) gated BOTH `willRemove()` AND `parentSubgraph.removeChild()`. When a consumed tile's subgraph was
  already invalidated by the parent cascade, the whole erase was skipped → its subgraph stayed a child of the
  parent → stale "2" rendered under the merged "4". Fix: guard ONLY `willRemove()` (the apply_tmpl traversal,
  genuinely unsafe); run `removeChild()` UNCONDITIONALLY (just unlinks; safe with the foreign-ref keeping
  storage alive). Verified: clean single-value tiles on device, no crash.
- **ANIMATION disabled (TileBoardView.swift, in repros/swift-canvas-spike):** the position
  `.animation(.interpolatingSpring, value:)` UAF (0.42=0x3ed70e9c over a Subgraph*) still CRASHES on aarch64
  cross-AOT (the foreign-ref fixes it on x86 JIT/desktop but not on-device). Disabled the position spring
  (kept the safe `.transition`) → tiles snap, board stays stable. Re-enable once aarch64-AOT is sorted.
REMAINING device follow-ons (separate from the integration): (1) **aarch64-AOT animation UAF** (the real fix
for the spring); (2) **proximity false-trigger** — the sensor blanks the panel OUTSIDE a call → auto-lock +
`input: touch SUPPRESSED (proximity blank)`, which blocks physical swipes (the auto-move diagnostic in
main.swift drives play without touch). Deploy = build-openswiftui-demo.sh → strip → component new →
`WANDR_AOT_TARGET=aarch64-linux-android wasm-android-host --install /tmp/osui-pkg` → push to
`$APPS/wandr.swiftui.demo/0.1.0` → `wandr-arbiter launch wandr.swiftui.demo` (kill the old pid first — the
arbiter foregrounds a stale instance otherwise).

## 🎯 ISOLATED (2026-06-23): the move-5-7 crash = an ANIMATION-VALUE WILD WRITE
DECISIVE bisection of the probe (10 fixed moves under bare `wasmtime`) pins the crash to ONE
modifier: **`.animation(.interpolatingSpring(stiffness:800, damping:200), value: position)`** on the
ForEach'd tile element in `TileBoardView.swift` (the eleev sources). Empirical proof:
- **Disable ALL transitions/animations → probe SURVIVES all 10 moves, exit 0.**
- Re-enable ONLY the TileView removal `.transition` (scale/opacity/modalSpring) → still SURVIVES.
- Re-enable ONLY the board `.animation(.interpolatingSpring, value: position)` → CRASHES (same addr).
So the corruptor is the **position spring**, i.e. OpenSwiftUI's `AnimatableFrameAttribute`
(`Animation/Animatable/AnimatableAttribute.swift:53`) — the per-frame **transactional**
`AnimatableAttribute<ViewFrame>` created by `Animatable._makeAnimatable` (`Animatable.swift:67`,
gated on `!inputs.animationsDisabled`). Its value type `ViewFrame` is CGFloat-based.

WHAT actually happens (corrected model — supersedes the "DynamicContainer.set_index garbage" framing
below, which was only ever a VICTIM): a transient interpolated **CGFloat ≈ 0.42** (float32
`0x3ED70EEC`/`0x3ED70E9C`; varies in low bits run-to-run = a live per-frame spring value, **NOT** a
Double — a Double 0.42 = `0x3FDA…`) gets **wild-written over a 4-byte `Subgraph*` pointer** living in
a Swift heap object (`_ViewList_Subgraph.subgraph`, ViewList.swift:2249 — a `let`, set once at init).
A later graph update then calls `forEach`/`apply_c`(self = 0.42) → `Subgraph::is_valid()` reads
`this+0x50` (`_invalidation_state`) → fault at `0x3ED70Exx`. The crash frame VARIES
(`set_index→DynamicContainerInfo` OR `willRemove→ForEachState.eraseItem→apply_tmpl`) and the victim
object MOVES with heap layout (adding any print relocates it) — the classic fingerprint of a **wild
write**, not a structural offset bug at a fixed site. Confirmed the `_ViewList_Subgraph` object is
otherwise intact (adjacent `refcount` reads sane =2) → a **targeted 4-byte write** lands exactly on
the `subgraph` slot.

RULED OUT decisively this session: **LayoutDescriptor / compare path is DEAD on wasm** — all 3 entry
points (`AGCompareValues` AGComparison.cpp:18, `AttributeType::compare_values` / `_partial`
AttributeType.h:68/89) `#if defined(__wasi__)` `return memcmp(...)` BEFORE reaching the
`sizeof(HeapObject)==0x10` math (LayoutDescriptor.cpp:564), and it only READS anyway → cannot be the
writer. So don't re-chase LayoutDescriptor. **NOT an 8-byte-CGFloat stride bug either** — the
node/body/value allocation + copy all use compiler-correct value-witness sizes (`vw_size`,
`getAlignmentMask`, `body_offset`); the standard write paths are fine.

## ⛓️ (2026-06-23) investigated a PAGE-REUSE UAF model — then FALSIFIED it (see next block)
A Compute-side detector in `value_set_internal` (Graph.cpp) fired: **`value_set` runs on
INVALIDATED subgraphs** (`is_valid()==false`) for the animated render attributes of just-removed
tiles — logged `type=DisplayList`, `type=Phase`, `type=ViewGeometry`. The model WAS (now disproven):
1. A tile merges → `eraseItem` → `Subgraph::invalidate_now` → `remove_subgraph` + the subgraph's
   **zone pages are freed back to the global `table`** (and the `Subgraph` C++ object is deleted).
2. But the removed tile's **`.animation(.interpolatingSpring, value: position)` attributes are
   `AsyncAttribute`s with PENDING updates** (`CoreGlue.nextUpdate`), and/or a still-live parent
   (display-list) holds an input edge into the dead node. Those updates/reads still fire.
3. The freed pages get **reused** — by another subgraph's allocation OR by Swift's heap (same
   underlying malloc) for a fresh `_ViewList_Subgraph`. The stale update resolves its `data::ptr`
   offsets through the page table into the reused page and **writes an interpolated CGFloat (~0.42)
   over a live `Subgraph*`** → later `forEach`(0.42) → `is_valid()` deref → SIGSEGV.

The `value_set`-on-invalid-subgraph events are REAL (the detector logged them) but — see the
falsification below — they are **a symptom, NOT the corruptor**. (`UpdateStack::update` gates input
pushes on `input_attribute.subgraph()->is_valid()` at UpdateStack.cpp:233, which lets some
invalidated-subgraph updates through once `is_valid()` is stale.) Why desktop survives but the probe
doesn't: the probe drives moves in a tight synchronous loop so removed-tile animations never settle
before the next removal; real frame-pacing lets them complete.

## ❌ FALSIFIED (2026-06-23): the crash is NOT reuse-based — OPTION 1 (dedicated arena) does NOT fix it
Prototyped the "isolate freed Compute memory" fix and ran a **sledgehammer falsification test**:
made BOTH `malloc_zone_free` (platform/malloc.h — the `alloc_persistent` path) AND
`table::dealloc_page_locked` (Table.cpp — the zone page table) **no-ops** under `#if __wasi__`, so
Compute **never reuses ANY freed heap memory**. The probe STILL crashes at the identical
`0x3ed70eec`. ⇒ The corruptor does **not** write into freed-then-reused memory. So:
- The "cross-subgraph page-reuse UAF" model above is WRONG as the crash cause.
- A **dedicated arena / memory-isolation fix cannot work** — don't build it.
- NOTE the page `table` ALREADY uses a private mmap arena (Table.cpp:53, MAP_ANON) recycled via a
  bitmap; only `alloc_persistent` (Table.cpp:39 → `malloc_zone_malloc` → plain `malloc` on wasm,
  platform/malloc.h:11) shares the system/Swift heap — and quarantining it changed nothing.

REVISED ROOT CAUSE: a **genuine WILD WRITE to a wrong COMPUTED address** in the
`.animation(.interpolatingSpring, value: position)` / `AnimatableFrameAttribute<ViewFrame>` path — an
interpolated CGFloat (~0.42) written to an address that lands on a live `Subgraph*` slot. Not a
reuse/UAF; a bad-address/offset computation (back to the original "type-confusion / field-offset"
thesis, now pinned to the animation path). Leading suspect: an **offset/indirect-attribute (mutable
indirect node) write** whose byte offset or base pointer is computed wrong on wasm32 (CGFloat=4 vs
Apple's 8) — the `.position`/`.frame` geometry uses offset attributes; audit the mutable-indirect-node
WRITE path (`AGGraphSetValue` on an offset attribute / indirect-node value write + its `offset()`),
NOT the plain `value_set` path (already shown correct).

ALSO ATTEMPTED + REVERTED (do NOT re-try — none fixed it): skip `value_set` on invalid subgraph; skip
the node UPDATE on invalid subgraph (UpdateStack.cpp:249); quarantine `malloc_zone_free`; quarantine
`dealloc_page_locked`. Trees are reverted clean (baseline crash reproduces).

OFFSET-ATTRIBUTE WRITE PATH — AUDITED CLEAN (2026-06-23, do not re-chase):
- `PointerOffset.of`/`.offset` (oag-fork PointerOffset.swift) computes byte offsets via a **runtime
  layout trick** (fake `Base` at `MemoryLayout<Base>.stride`, take `&base.member`, subtract) → uses
  the compiler's ACTUAL wasm32 field layout, correct, NOT an 8-byte assumption.
- Offset/indirect attributes are **read-only projections**: `AGGraphSetValue` (AGGraph.cpp:569)
  `precondition_failure("non-direct attribute id")` rejects indirect attrs; `IndirectNode::modify`
  only updates metadata; `resolve_slow` (AttributeID.cpp:85) just accumulates `offset`. No write
  vector here.
- The animation path uses `AnimationListener` (Swift class refs) + `CoreGlue.nextUpdate`, NOT
  `AGSubgraphAddObserver` → the stored-observer-closure-retention angle is also off-path.

RULED-OUT SUMMARY (all clean — the wild write is NONE of these): LayoutDescriptor/compare; plain
`value_set`/node-body/value alloc+copy (correct vw_size); memory reuse (page table + alloc_persistent
quarantine = still crashes); offset/indirect-attribute writes; subgraph-observer closures.

ARTIFACT HYPOTHESIS — FALSIFIED (2026-06-23): added a `wandrAdvanceTime(dt)` SPI
(`host.render(interval:dt,…)` advances `currentTimestamp`, ViewRendererHost.swift:198) and drove 16×
0.05s settle-frames between moves so removed-tile springs actually RUN + complete. STILL crashes
(same `0x3ed70edc`). So the probe's frozen-t=0 tight loop is NOT the cause — the crash happens during
ACTIVE spring interpolation, independent of pacing. ⇒ NOT a probe-stress artifact; the device would
hit this during animations too (it currently trips the SEPARATE `pow` SIGILL first, also
animation-triggered). (SPI + probe pacing reverted; trees clean.)

## 🎯 WRITE-CATCHER BUILT (2026-06-23) — corruptor localized to GeometryReader content re-eval
Built a software watchpoint (the runtime write-catcher): Swift registers each `_ViewList_Subgraph`
+ `DynamicContainer.ItemInfo` `subgraph`-slot ADDRESS (made the `let` a `var`; `@_extern(c)` to a
C++ registry in Subgraph.cpp — NOTE `@_silgen_name` mislowers the C ABI on wasm → `signature_mismatch`,
use `@_extern(c)`); `wandr_watch_check` scans all slots after every `attribute_type.update`
(UpdateStack.cpp:260) and traps when a slot goes out-of-bounds (valid `Subgraph*` < memsize; the
clobber writes a value ≥ memsize). Findings:
- The watchpoint CAUGHT real corrupting writes during **`GeometryReader.Child.updateValue`**
  (`GeometryReader.swift:80`, a `StatefulRule, AsyncAttribute` whose Value is
  `_VariadicView.Tree<_LayoutRoot<GeometryReaderLayout>, Content>`) — specifically during its
  `content(proxy)` re-evaluation (clean at a `geo-start` check, clobbered at `geo-after-content`).
- The clobber VALUES are string/flag garbage: `0x57454956` = ASCII **"VIEW"** (little-endian),
  `0xa0000000`, `0x0` — i.e. a buffer write spraying non-pointer bytes over subgraph-pointer memory.
- The C++ per-`attribute_type.update` check did NOT fire before `geo-after-content` → the write is in
  the **Swift view-rebuild closure execution** (`$view.syncMainIfReferences { v in v.content(proxy) }`
  / `withObservation`), NOT a nested sub-attribute update.
- With deinit-DEREGISTRATION (watch only LIVE objects), those catches stop AND the deterministic crash
  reads a `0.42` **child subgraph pointer in a live Subgraph's `_children`** (Compute C++, malloc'd via
  `details::realloc_vector`), reached during `willRemove`'s `forEach` traversal — NOT `item.subgraph`
  (which is valid at `eraseItem` entry). So the spray hits BOTH freed Swift item slots AND live
  Compute `_children` entries; the crash victim varies with heap layout.
⇒ ROOT (localized): GeometryReader's animation-driven content re-evaluation performs a wasm32
buffer/size-wrong write that sprays garbage over adjacent subgraph-pointer memory (Swift item slots +
Compute `_children`).

### Watchpoint round 2 (2026-06-23) — narrowed further; two hypotheses KILLED
Extended the watchpoint: also registers every live `AG::Subgraph` (ctor/dtor) and scans each one's
`_children` for an out-of-bounds child pointer (`wandr_scan_children`, robust to vector realloc);
added high-freq checks (`add_child`, item inits). Results:
- **NOT premature-free of the Swift item objects**: pinned every `_ViewList_Subgraph` +
  `DynamicContainer.ItemInfo` alive forever (`_wandrItemPin.append(self)`, never deinit) → STILL
  crashes at `0x3ed70eec`. So the live item's `subgraph` field (or the `info.items` array buffer) is
  wild-written; the object is alive.
- **The corruption is WITHIN the crashing update**, between check points: at `eraseItem` entry
  `item.subgraph` is VALID, then `item.subgraph.willRemove()` traverses and hits a `0.42` subgraph;
  the post-`attribute_type.update` check + the `add_child`/init checks never fire in between. So the
  spray happens during a single attribute update's Swift execution (the GeometryReader content
  rebuild / the ForEachState.Info / DynamicContainerInfo updateValue), and the victim varies with
  heap layout (item slot / `_children` entry / `info.items` buffer) — a buffer-overflow SPRAY.
- Clobber values seen: `0x57454956`="VIEW", `0xa0000000`, `0x0`, `0x3ed70eec` (the crash).
LIMIT of the printf/trap-watchpoint technique: the write is within-update + heap-sensitive +
sprays, so update-boundary checks miss it and each rebuild shifts the victim. NEXT needs a finer
technique: a true wasm HW watchpoint (wasmtime GDB-JIT `--debug` + `watch *addr`) on a victim slot
found from a coredump, OR sub-update Swift instrumentation of the specific content-rebuild path
(`_VariadicView.Tree._makeViewList` / the ViewList construction during `v.content(proxy)`).
(Old NEXT, partly done: (a) value-read vs content-build split — checks added but heap-shift moved the
catch; (b) `_children` watch — DONE, didn't catch the within-update write; (c) audit
`syncMainIfReferences`/`valueAndFlags` value-read buffer — DONE, **CLEAN**: `valueAndFlags`
(Compute Attribute.swift:166) reads the small `GeometryReader` value via
`__AGGraphGetValue(...).assumingMemoryBound(to: Value.self).pointee` — correct size. So the overflow
is in `v.content(proxy)` = the **view CONSTRUCTION** (rebuilding the `_VariadicView.Tree`/ViewList
hierarchy), NOT the value read.)

### Watchpoint round 3 (2026-06-23) — _makeViewList sub-step instrumentation → HEAP-ROULETTE wall
Instrumented `_VariadicView.makeViewList` sub-steps (`body`/`makeAttribute`/`makeBody`/
`makeDebuggableViewList`), a `DynamicContainerInfo.updateValue` `info.items[]` element scan, and an
in-loop `set_index` target/element check. NONE caught the write; each added check just **shifted the
symptom**. Across the session the SAME corruption surfaced as FOUR different deterministic-per-build
crashes, all "a corrupted subgraph in the view-list update path":
1. `willRemove → forEach → Subgraph::is_valid(0.42 child)` (a `_children` entry)
2. `DynamicContainerInfo.updateValue → AGSubgraphSetIndex(0.42 item.subgraph)` (the `info.items`
   buffer — confirmed `info.items[i]` all VALID right before the loop, so `target` reads OOB / a
   sprayed element)
3. `DynamicViewList.updateValue → _ViewList_Subgraph.isValid(0.42 subgraph)`
4. `DynamicViewList.updateValue → AGSubgraphAddChild` precondition "child subgraph must have same
   graph" (a Subgraph OBJECT whose `_graph` field is clobbered)
⇒ CONCLUSION: a single malloc-heap buffer-overflow SPRAY (during the GeometryReader-animation-driven
view-list update/construction) corrupts whatever subgraph-related memory is adjacent; the symptom is
pure heap roulette. **The printf/trap-watchpoint technique is EXHAUSTED** — within-update + spray +
heap-sensitive means boundary checks miss it and instrumentation relocates it. The ONLY reliable way
to the exact instruction is a true wasm HW watchpoint (wasmtime GDB-JIT `--debug`, `watch *addr` on a
victim slot located from a coredump) OR a sanitizer build. Symptom #4 (a clean precondition, not a
wild deref) is the best next anchor: it deterministically catches a clobbered Subgraph `_graph` — set
a HW watchpoint on that subgraph object's `_graph` and re-run.

### ⛔ CROSS-MODULE WALL (2026-06-23 latest) — foreign-ref members don't propagate through the OAG shim
PROGRESS: the static-member fix WORKED — the `Compute` module now COMPILES with foreign-ref Subgraph
(swift_name dropped from the 6 statics in AGSubgraph.h/AGAttribute.h; redefined in
Compute/Graph/Subgraph.swift `extension Subgraph` over the `__AGSubgraph…` refined funcs:
typealias Flags = AGAttributeFlags; static current get/set; currentGraphContext; shouldRecordTree;
setShouldRecordTree). Build PROCEEDED past Compute into OpenSwiftUI → 4700 NEW errors.
ROOT of the 4700: OpenSwiftUICore sees `Subgraph` (aka ComputeCxx.AGSubgraphStorage) with NO members
at all (instance too: graph/addChild/invalidate/init/…). Cause = Swift C++ interop only exposes a
FOREIGN-REFERENCE class's members in modules that DIRECTLY `import` the C++ module (ComputeCxx).
Chain: OpenSwiftUICore `package import OpenAttributeGraphShims` → `@_exported import OpenAttributeGraph`
(+ COMPUTE=1 Adapter/Compute.swift). The TYPE flows (typealias) but the C++ MEMBERS don't cross the
shim boundary. Upstream jcmosc/Compute never hits this (its Subgraph users import `Compute` directly,
no OAG shim). The old value-type import propagated members cross-module; the foreign-ref class does NOT.
NEXT (structural, fresh session): make the C++ foreign-ref members propagate to OpenSwiftUI. Options to
try, cheapest first: (a) `@_exported import ComputeCxx` (and/or `Compute`) in OpenAttributeGraphShims
(OAGShims.swift / Adapter/Compute.swift) so members flow; (b) if Swift won't re-export C++ members,
re-declare the needed Subgraph instance methods as Swift shims in OpenAttributeGraph(Shims) — same
trick as the statics, but for instance members, wrapping the `__AGSubgraph…(self:)` refined funcs;
(c) verify a single `import ComputeCxx` in one failing OpenSwiftUI file resolves `Subgraph.graph` —
if NOT, it's a harder interop limitation and reconsider. Patches saved (compute-wasm.patch = WIP
foreign-ref + static-member extension; oswui patch = hacks removed). This is the right hand-off point.

### 🔑 BREAKTHROUGH + WIP (2026-06-23 latest) — foreign-reference fix, grounded in upstream
READ-SOURCE-FIRST PAYOFF (user pointed at github.com/jcmosc/Compute):
- Upstream jcmosc/Compute declares Subgraph IDENTICALLY (CF type, arc_cf_code_audited, GetTypeID) +
  IDENTICAL C++ lifecycle (CFRetain/CFRelease only on current-subgraph; add_child/add_subgraph use
  raw ptrs) + Subgraph.swift does NO explicit retain/release. It works on Linux/macOS PURELY because
  Swift's ClangImporter ARC-manages the CF type there. So: our C++ already matches upstream — the
  C-refcount pass was the WRONG direction; the gap is wasm not ARC-managing the CF type.
- Upstream branch `fix-compatibility-tests` ADDS `IAG_SWIFT_SHARED_REFERENCE` + Utilities/SwiftBridging.h
  (SWIFT_SHARED_REFERENCE / RETURNS_RETAINED / RETURNS_UNRETAINED) = the foreign-reference mechanism
  for the OSS toolchain. Not yet applied to Subgraph upstream (WIP), but blesses the approach + macro.

IMPLEMENTED (compiles to the IMPORT stage; WIP):
- AGSubgraph.h #if __wasi__: `typedef struct AG_SWIFT_SHARED_REFERENCE(AGSubgraphRetainRef,
  AGSubgraphReleaseRef) AGSubgraphStorage *AGSubgraphRef AG_SWIFT_NAME(Subgraph)` →
  Subgraph imports as a Swift MANAGED CLASS. retain/release = CFRetain/CFRelease wrappers
  (AGSubgraph.cpp, replaced the wandr_subgraph_retain/release helpers). Added AG_SWIFT_RETURNS_RETAINED
  on Create/Create2, AG_SWIFT_RETURNS_UNRETAINED on GetCurrent (AGBase.h has the macros).
- All `AG_SWIFT_NAME(AGSubgraphRef.x)` → `Subgraph.x` (AGSubgraph.h + AGAttribute.h Flags).
- REMOVED the C++/Swift hacks (item-retain in ViewList/DynamicContainer; the wandr_subgraph_* funcs).
  (graph-retain already reverted; robust isValid + isValid guards still present, harmless, remove later.)

REMAINING (bounded, ~6 members) — the ONE blocker: foreign-ref classes import INSTANCE members
(self:) fine but NOT C static members / nested types via swift_name. So 111 errors, all from:
`Subgraph.Flags` (66), `.current` (18), `.shouldRecordTree` (18), `.currentGraphContext` (8).
RECIPE to finish:
1. In AGSubgraph.h REMOVE the swift_name from the STATIC decls (keep AG_REFINED_FOR_SWIFT + the
   RETURNS_* annotations for correct ownership!): AGSubgraphGetTypeID, AGSubgraphGetCurrent,
   AGSubgraphSetCurrent, AGSubgraphGetCurrentGraphContext, AGSubgraphShouldRecordTree,
   AGSubgraphSetShouldRecordTree → they import as refined free funcs `__AGSubgraph…`.
2. In AGAttribute.h REMOVE `AG_SWIFT_NAME(Subgraph.Flags)` from the AGAttributeFlags enum → it
   imports as `AGAttributeFlags`.
3. Add a Swift `extension Subgraph` (Compute/Graph/Subgraph.swift) redefining the statics by calling
   the refined free funcs (proper ownership via the importer — do NOT @_silgen_name a foreign-ref
   RETURN, that bypasses ownership → over-release): `typealias Flags = AGAttributeFlags`;
   `static var current: Subgraph? { get { __AGSubgraphGetCurrent() } set { __AGSubgraphSetCurrent(newValue) } }`;
   `static var currentGraphContext`; `static var shouldRecordTree`; `static func setShouldRecordTree()`;
   `static var typeID`. Watch the leading-underscore refined names (swift_private = `__` prefix).
4. Rebuild probe (60-move ProbeApp = base×6, deterministic). Expect import errors to clear; then the
   UAF should be GONE (ARC manages every ref incl. structs). If survives 60 moves 5/5 → cross-AOT to
   device + retest. Then DELETE the now-redundant robust-isValid + isValid guards to match upstream.
Risk after: `==`/Equatable or other value-uses of Subgraph may surface (foreign-ref = class); fix as
they appear. Patches saved (compute-wasm.patch has the foreign-ref; it's WIP/non-compiling).

### ⚠️ STATUS CORRECTION (2026-06-23 later) — NOT fixed; C++ refcount pass hit a hard wall
Earlier "device plays / 5/5 desktop" was a HEAP-DEPENDENT FALSE POSITIVE: the 10-move probe got
lucky on heap layout. Extended probe (60 moves, deterministic) crashes at **move 5** (4/4 runs).
Crash progression as fixes were added: original move 3-4 (subgraph storage UAF) → +item-retain
move 5 (DisplayList.Effect box corruption) → +graph-retain move 7 (Subgraph::_children OOB in
remove_child). **graph-retain REVERTED** — its CFRelease in remove_subgraph runs INSIDE
invalidate_now's walk (line 23) and can free the storage re-entrantly mid-invalidation (unsafe).
Current state = item-retain only (`_ViewList_Subgraph` + `DynamicContainer.ItemInfo` CFRetain/release)
+ robust AGSubgraphIsValid + isValid guards = deterministic crash at move 5.

**DEFINITIVE WALL (why C++ refcount can't converge):** move-5/move-7 victims are both
STRUCT-HELD `Subgraph` refs (a subgraph inside `DisplayList.Effect`; `parentSubgraph` in
ForEach/container). The ~20 Swift `Subgraph` properties — several in STRUCTS (`ConditionalContent.Info`,
`GraphHost.Data`, `ViewListContent`, `VariadicView.Tree`) — hold the CF AGSubgraphRef BY VALUE as an
UNMANAGED type (no CF refcount participation, no deinit). No C++-side refcounting can make them keep
the storage alive; the moment a struct ref outlives the storage it dangles. Unfixable from C++.

**THE FIX = importer foreign-reference type** (revises the earlier "breaks value semantics"
pessimism): if `Subgraph` is imported as a MANAGED reference type, Swift's automatic memberwise ARC
retains/releases it inside structs FOR FREE (a struct releases a class-typed field on destruction,
no deinit needed). Plan: make `struct AGSubgraphStorage` a Swift foreign reference type on wasm via
`swift_attr("import_reference"/"retain:…"/"release:…")` (retain/release = CFRetain/CFRelease
wrappers), DEDICATED macro (don't touch shared AG_BRIDGED_TYPE / AGGraphRef), then DELETE the manual
item-retain (auto-managed). Risks: foreign-ref restrictions (no by-value), factory-init mapping for
AGSubgraphCreate2 (+1 ownership), possible `==`/value-usage breakage — expect several compile
iterations. Deterministic repro for it = probe (ProbeApp.swift seq = base×6 = 60 moves).

### ✅✅✅ ROOT FIX (2026-06-23 latest) — CF/Swift-ARC retain; desktop solid + device STABLE
DEEPER ROOT CONFIRMED: `AGBase.h` `AG_BRIDGED_TYPE(id)` = `__attribute__((objc_bridge(id)))` on
Apple (so Swift ARC retains the AGSubgraphStorage while an item holds `var subgraph: Subgraph`) but
**EMPTY on wasm** → `Subgraph`/`AGSubgraphRef` is imported UNMANAGED → Swift items hold the storage
with no retain → the storage's CF refcount excludes item refs → freed (CF refcount→0) when removed
from graph/parent while items still point at it = the use-after-free.
PROPER FIX (replaces the from_cf-null band-aid, which crashed the device's fuller render path at
fault 0x10):
1. **Item-level retain/release** — `_ViewList_Subgraph` (covers DynamicViewList/ForEachState items)
   + `DynamicContainer.ItemInfo` call `wandr_subgraph_retain`/`release` (CFRetain/CFRelease wrappers
   in AGSubgraph.cpp, #if __wasi__) in init/deinit → replicates Apple's ARC. Keeps item subgraphs
   alive (no UAF).
2. **Robust `AGSubgraphIsValid`** — some non-item refs (e.g. `DynamicViewList.parentSubgraph`) live
   in STRUCTS that can't deinit to release, so they can still be freed; `isValid` bounds-checks the
   ptr and returns false (correct: a freed ref IS invalid) instead of faulting. Callers already
   guard on isValid.
3. **isValid guards** kept: DynamicViewList reuse loop, ForEachState.eraseItem, DynamicContainer
   set_index loop — skip invalidated subgraphs (now SAFE because the storage stays alive).
4. **from_cf / AGSubgraphSetIndex REVERTED to original** (band-aids removed; from_cf no longer
   returns null → no unguarded-caller 0x10 crash).
VERIFIED: desktop 5/5 deterministic + 3/3 random SURVIVE all 10 moves, exit 0, renders. DEVICE
(Pixel 2 XL, cross-AOT aarch64): app launches + runs STABLE (alive 17s+, **0 crashes**) where the
band-aid crash-looped at 0x10. Behind the wandr keyguard (adb `input` can't dismiss the evdev-direct
--no-art stack) → final interactive play needs a physical swipe (user visual check).
Deploy: build-openswiftui-demo.sh → strip → component new → `WANDR_AOT_TARGET=aarch64-linux-android
wasm-android-host --install` → push to /data/local/tmp/wandr-apps/apps → `wandr-arbiter launch
wandr.swiftui.demo`. (Incident: a git-checkout reverted the uncommitted apply_c patch — re-added.)

### ✅✅ FIXED (2026-06-23) — eleev 2048 plays ALL 10 moves, renders, exit 0 (det + random)
ROOT CAUSE: a **use-after-free of an `AGSubgraphStorage`**. `Subgraph` = `AGSubgraphRef` =
`AGSubgraphStorage*`, a CF-bridged (`AG_BRIDGED_TYPE(id)`) type. On wasm an item's CF-typed
`subgraph` ref does NOT keep the storage alive the way it should, so when a subgraph is
invalidated/removed the storage's CF refcount hits 0 → finalized + freed → its memory reused (e.g.
for a `[ViewGeometry]` array, so `storage->subgraph` reads back a coordinate like `0x42bb3333`=93.6).
Stale Swift items still hold the `AGSubgraphRef`; the next op on them (`add_child`, `set_index`,
`willRemove→apply_tmpl`, `is_valid`) dereferences the garbage → SIGSEGV. The "coordinate sprayed
over a subgraph slot" was the REUSED memory, not a write — which is why chasing a single
"corrupting write" never converged (and the victim address moved every build).

THE FIX (4 changes, all shipped to compute-wasm.patch + openswiftui-phase1-wip.patch):
1. **`Subgraph::from_cf` (Subgraph.cpp)** — choke point: return `nullptr` when `storage->subgraph`
   is out of linear-memory range (a freed+reused storage), so every caller sees "invalidated"
   instead of dereferencing garbage. THIS is the load-bearing fix.
2. **`AGSubgraphIsValid` (AGSubgraph.cpp)** — bounds-check the subgraph ptr → return false (don't
   fault) for a freed storage, so `isValid` guards work.
3. **`AGSubgraphSetIndex` (AGSubgraph.cpp)** — no-op (not precondition_failure) on a null/invalid
   subgraph.
4. **OpenSwiftUI item guards** — `DynamicViewList.updateValue` reuse loop (DynamicView.swift:156)
   and `ForEachState.eraseItem` (ForEach.swift) were missing the `item.isValid` guard that the
   `lastItem` path / `ViewList.Subgraph.remove(from:)` already have. Added it.
VERIFIED: clean build (ALL diagnostic instrumentation + the item-pin removed) → 5/5 deterministic +
2/2 random runs SURVIVE all 10 moves, exit 0, board renders correctly. Cranelift≡Winch earlier
proved it's a guest bug not wasmtime; not WasmGC/DRC (that's the Kotlin path).
DEEPER ROOT (not yet fixed, defensive fix masks it): WHY the CF-typed `subgraph` ref doesn't retain
the storage on wasm — likely Swift-ARC-vs-CF-refcount on the wasi CoreFoundation. `from_cf` null-
guard is a correct defensive behavior (removed subgraphs read as invalid) so 2048 is fully playable;
the retain investigation can continue separately. (Incident note: a `git checkout` of Subgraph.cpp
during cleanup reverted the uncommitted `apply_tmpl`/`apply_c` patch — re-added from compute-wasm.patch.)

### Perturbation-immune victim trace (2026-06-23) — REFRAME: not a single write, it's REUSE
Built a perturbation-immune tracer: same fixed binary, victim address passed via env (`WANDR_VICTIM`),
read at every setGeometry/childGeometries/value_set/attribute-update checkpoint. KEY trick: keep the
env-var WIDTH constant (10-char `0x%08x`) across the learn-run and the trace-run so the wasi environ
size — and thus the heap — is identical (adding/resizing an env var shifts the heap and moves the
victim). Findings:
- Proved it's a GUEST bug, not wasmtime: **Cranelift ≡ Winch** produce byte-identical crashes
  (same move, victim addr, value). Not wasmtime GC either — Swift is linear-memory wasip1 + ARC, no
  WasmGC/DRC (that's the Kotlin path).
- The flip to 0x42bb3333 is detected at `value_set:ENTRY type=Array<ViewGeometry>` — i.e. when the
  `LayoutChildGeometries` Rule stores its `[ViewGeometry]` output, AFTER the value compute
  (childGeometries cg-* checkpoints are all valid), in the tiny window between compute and store.
- **REFRAME (important):** the victim ADDRESS is heavily REUSED/REASSIGNED — over a run the same
  slot cycles through many different valid subgraph pointers (0x247c9b0, 0x2738334, …) and ends as a
  ViewGeometry coordinate (0x42bb3333). So the watchpoint's "clobber" is partly **benign memory
  reuse** (a ViewGeometry array allocated where a registered Item slot used to be), i.e. FALSE
  POSITIVES — which is why a single corrupting "write" never pinned. The TRUE signal is the
  downstream USE of a stale subgraph: `add_child` (child._graph=NULL, FIXED via isValid) and
  `ForEachState.eraseItem → willRemove → apply_tmpl(this=coordinate)` = a **use-after-free of an
  AGSubgraphStorage** (storage freed, memory reused for a ViewGeometry array, stale Item ref used).
NEXT (correct lead): drop the noisy watchpoint; go from the REAL crash (apply_tmpl via
ForEachState.eraseItem, DWARF-backtraced) and find why the AGSubgraphStorage is freed while an Item
still references it — the subgraph CF-refcount / invalidation path (over-release or missing retain),
analogous to the DynamicViewList isValid fix but on the ForEachState/eraseItem side.
Trace infra (env-victim, FLIP/CKPT, wandr_au_check at update entry/exit) is in
Subgraph.cpp/Graph.cpp/AttributeType.h/Layout.swift — gate `WANDR_VICTIM`.

### DWARF-at-value_set follow-up (2026-06-23) — two buffer candidates RULED OUT by overlap check
Re-enabled the value_set watchpoint + DWARF. The clobber is detected at the `LayoutChildGeometries`
Rule's `value_set` of its `[ViewGeometry]` output (backtrace: `value_set_internal ←
AGGraphSetOutputValue ← Rule._update ← ... ← LayoutChildGeometries.value ← childGeometries ←
GeometryReaderLayout.placeSubviews ← setGeometry`). To find the OVERFLOWING buffer, added a C helper
`wandr_check_overlap(lo,hi)` that scans the live registered ItemInfo.subgraph slots and flags
containment/adjacency. RESULTS:
- **`geometrys` array buffer does NOT overlap or adjoin the victim** — checked at childGeometries
  alloc time; the only adjacencies are normal interleaving (an ItemInfo sits 64–120B *before* a
  geometrys buffer; the array write goes forward, away from it). So the `geometrys[index]` write is
  NOT the spray (re-confirmed: its dest is its own buffer).
- **value_set node-storage (`value_dest`) does NOT overlap a live ItemInfo** either (`vw_assignWithCopy`
  dest checked vs all slots — no `[WANDR-VS-OVERLAP]`). So the value-store copy isn't the spray.
- **Victim address MOVES every build** (0x276a560 → 0x2733e70 → …) — perturbation-sensitive ⇒ it's a
  heap-adjacent buffer overflow where which ItemInfo gets hit depends on layout. The corrupting
  buffer is some OTHER allocation made during the layout (a nested `dimensions()` measurement temp, a
  layout `cache`, or a Compute-side copy in `mark_changed`), overflowing into an adjacent ItemInfo.
TOOLING WALL: pinning the exact overflowing write needs an instruction-level DATA watchpoint.
wasmtime has NONE that fit: gdbstub `Z2`=UNIMPLEMENTED; per-call breakpoints too slow (~1s/hit);
`-W wmemcheck` not in our binary AND wouldn't catch it (it flags writes to *unallocated/freed* memory,
not overflow into an adjacent *live* malloc block — no redzones). The software watchpoint is a POLL
(catches at the next value_set, mislabels the type). FIX #1 (DynamicViewList isValid) stands and is
real. Remaining = this heap-adjacent layout-buffer overflow.

### ⭐ DWARF-BACKTRACE BREAKTHROUGH (2026-06-23 latest) — precise crash localization + 1 real fix
User push: "enable wasmtime debug/stack/backtrace." KEY: `wasmtime run -D debug-info=y
-D max-backtrace=120` on the DEBUG-built probe (`-Xcc -g`) gives **fully symbolicated Swift+C++
backtraces** at the real abort/trap — collapse the repeated `UpdateStack::update` frames
(`awk '!/UpdateStack::update/||!seen[$0]++'`) to see the leaf. This finally named exact functions.
(Note: `-W wmemcheck=y` is NOT in our prebuilt wasmtime binary — needs a wasmtime rebuild with the
`wmemcheck` feature, and wouldn't catch this anyway since it's an overflow into an *adjacent live*
block. `WasmBacktrace` is a Rust embedding API, same info as the CLI backtrace — not worth a custom
host.)

FINDINGS (bypass + DWARF combined):
- **Bypass proved the corruptor is the dynamic `ForEach(matrix.flatten(), id:\.tile.id)`** —
  `WANDR_NO_TILES=1` (skip it) → SURVIVES all 10 moves, exit 0. Animation/transitions/box-offset/
  StackLayout/TLS/set_index/displayMap/removeChild all RULED OUT (each bypassed, crash persisted).
- **FIX #1 (real bug, applied):** `DynamicViewList.updateValue` (DynamicView.swift:156) reuse loop
  guarded only on `item.matches(...)`, while the `lastItem` fast-path (line 146) checks
  `matches && isValid`. A parent-subgraph cascade can invalidate an item's subgraph (zeroing
  `_graph`) without calling `Item.invalidate()` (which removes it from `allItems`), so a stale item
  lingers and gets re-`addChild`'d → `Subgraph::add_child` precondition "child subgraph must have
  same graph" (`child._graph==0x0`). Fix = add `, item.isValid` to the loop guard. **Moved the
  crash MOVE 3 → MOVE 4.**
- **REMAINING ROOT (move 4):** a **buffer overflow sprays a ViewGeometry coordinate over subgraph
  slots**. DWARF leaf: `ForEachState.eraseItem → subgraph.willRemove() → AGSubgraphApplyC →
  apply_tmpl(this=0x3e1e96e5)` — `this` is a FLOAT (~0.155, a coord); brute-scan finds that same
  coord legitimately filling a 32-byte-stride ViewGeometry array AND clobbering an ItemInfo.subgraph
  slot. Watchpoint (when enabled) catches it "during setGeometry" via
  `GeometryReaderLayout.placeSubviews → LayoutSubview.place → PlacementData.setGeometry`. BUT
  `setGeometry`'s `geometrys[index]` write is bounds-checked + lands in its own buffer — so the
  spray is a SIDE-EFFECT of the place→setGeometry execution (CoW/realloc of `geometrys` landing on a
  freed-and-reused subgraph storage, or a stale `placementData` thread-local from the AsyncAttribute
  `LayoutChildGeometries`), NOT the array write. This is the same overflow that earlier manifested
  as 0.42/93.6/0xf0000001 — heap-adjacency-sensitive (perturbation shifts victim+move).
NEXT: re-run `-D debug-info=y` with the value_set watchpoint to get the DWARF backtrace at the
EXACT value_set that coincides with the spray (named a Rule before); read the `geometrys` array
lifetime in `ViewLayoutEngine.childGeometries` (Layout.swift:1247) for a CoW/realloc-over-freed-
storage or async-stale-`placementData` overflow.

### CODE-READ of the layout-engine geometry path (2026-06-23 latest) — two big suspects RULED OUT
Read the full layout geometry path end-to-end. Findings (both ELIMINATED, which redirects the hunt):
- **Swift layout-engine geometry path is CORRECT on wasm32.** `StackLayout` (ZStack→the 2048 board)
  builds `children` at exactly `proxies.count`; `childrenPtr[i]` writes are bounds-checked (debug);
  `UnsafeMutableBufferProjectionPointer` (non-bounds-checked, used for `fittingOrder`/`layoutPriority`)
  computes addresses with wasm32-correct `MemoryLayout<Scene>.stride` + `pointer(to: keyPath)`;
  `insertionSort` keeps indices in range; `CGPoint[axis]` is `x:y` (no offset math); `setGeometry`'s
  `geometrys[index]` lands in its own buffer. No spottable overflow.
- **C++ boxed-payload offset is CORRECT on wasm32 (was the prime suspect — now dead).** The pattern
  `offset=(sizeof(::swift::HeapObject)+align)&~align` (Metadata.cpp:827 `project_value`,
  AGType.cpp:356/414 `AGTypeApply[Mutable]EnumData`, LayoutDescriptor.cpp:564) uses
  `sizeof(::swift::HeapObject)` which on wasm32 = **8** (verified via startup print). 8 is RIGHT:
  `RefCount.h:221` `RefCountBitsInt<RefCountIsInline,4>→uint32_t` ⇒ Swift uses a 4-byte inline refcount
  on 4-byte-pointer targets ⇒ HeapObject=metadata(4)+refcount(4)=8, payload at +8. The `[wasm32]`-
  silenced `static_assert(==0x10)` was the Apple-64 value; using the live `sizeof` is correct.
  AGCompareValues is a pure `memcmp` bypass (LayoutDescriptor compare path dead). NOT the bug.
- **Also already excluded:** the `placementData`/`_threadLayoutData` TLS; the `geometrys[index]` write.
⇒ Corruptor fires DURING layout/render attribute updates (confirmed across binaries) but is **neither
a layout-buffer overflow nor a box/HeapObject-offset bug**. NEXT to read: (a) DisplayList *build*
(the "during DisplayList" catch + 0.42 = an interpolated animation fraction); (b) the
animation/`AnimatableData` interpolation value STORE; (c) a DynamicContainer/ForEach *logic* bug
(history: "uninitialized _data", "willRemove UAF") — a stale index/ref, not a wasm32 layout bug.

### 🎯 ROOT-CAUSE SUBSYSTEM PINNED (2026-06-23 latest): it's a LAYOUT bug, NOT animation
IMPORTANT REFRAME (user caught my imprecision): the animation is the **TRIGGER**, not the bug.
`.animation(.interpolatingSpring, value: position)` drives a **per-frame re-layout**; disabling it
only stops the re-layout that exposes the defect. The DEFECT is in the **layout geometry
computation**. The deterministic software watchpoint (storage-slot scan) traced the corrupting store
through: `LayoutChildGeometries.value` (LayoutView.swift:188) → `LayoutComputer.childGeometries` →
`ViewLayoutEngine.childGeometries` (Layout.swift:1247) → `layout.placeSubviews(...)` →
`LayoutSubview.place(in:)` (Layout.swift:1717) → `PlacementData.setGeometry` (Layout.swift:1753). The
clobber lands DURING `placeSubviews` (clean at a pre-check, dirty at post), writing a geometry value
(`0x42bb3333`=93.6 / `0x3ed70e9c`=0.42 / `0xf0000001`) over a LIVE `AGSubgraphStorage.subgraph` field
(`AGSubgraph-Private.h:19`; the field `AGSubgraphApplyC`→`from_cf` reads → SIGSEGV later). It is a
**buffer-overflow / wrong-address write in the layout geometry computation, not the `geometrys[index]`
array write** (that goes correctly to its own buffer, far from the victim). Ruled out: the
`placementData` thread-local (`TLS.c` `_Thread_local`→plain `static` made NO difference).

### gdbstub can NOT pin the exact instruction (tooling limits, confirmed)
With the deterministic repro + working lldb/gdbstub I tried the perturbation-free breakpoint route:
- **No data/HW watchpoints** (`Z2` → `UNIMPLEMENTED`).
- **Per-`setGeometry` breakpoint + memory-read-V auto-continue is too SLOW** — ~1 s per hit via the
  gdbstub round-trip; 266 hits in 280 s, never reached the crash (move 4). (V read works:
  `memory read -f x -s 4 -c 1 <addr>` → `0x02754040: 0x0274fb00`; the breakpoint resolves; it's just
  too slow to traverse thousands of calls.)
- **Conditional breakpoints / any expression eval CRASH lldb** (ObjC-codegen bug in this build).
⇒ The gdbstub is great for *stopping at the crash with source info* but cannot efficiently *find the
store*. The **software watchpoint is the working tool** (it pinned the subsystem); its only downside
is instrumentation perturbs which exact slot/frame is hit (heap-roulette within a deterministic run).
NEXT (runtime tooling exhausted for finer pinning): READ the layout geometry code —
`ViewLayoutEngine.childGeometries` (Layout.swift:1247), the active `layout.placeSubviews` + its
`cache`, `LayoutSubview.dimensions(in:)`, and `PlacementData` — for the wasm32 buffer/size/offset
bug. The victim (`AGSubgraphStorage`, ~12–16B CF object) is allocated adjacent to a geometry buffer
that overruns it.

### 🔑 BREAKTHROUGH (2026-06-23 late): DETERMINISTIC repro + working wasm debugger
Two big unlocks that make this bug tractable (it was "heap-roulette, uncatchable" before):
1. **DETERMINISTIC REPRO.** Two nondeterminism sources fixed → identical crash every run
   (fault `0x3ed70eec`, MOVE 4, `is_valid`):
   - The eleev 2048 uses `Int.random`/`randomElement` for tile spawns → replaced with a seeded LCG
     in `GameLogic.swift` (`_wandrRand`, `#if os(WASI)`), seed `0x2545F4914F6CDD1D`.
   - Swift randomizes Dictionary/Set hash seeds per-process → run with env
     **`SWIFT_DETERMINISTIC_HASHING=1`** (`wasmtime run --env SWIFT_DETERMINISTIC_HASHING=1 …`).
   ⇒ For a GIVEN binary the victim address + crash are now byte-stable across runs. (Adding/removing
   instrumentation still shifts layout, but a fixed binary is fully reproducible.)
2. **wasm source-level debugger WORKS.** Fixed `lldb`: the swiftly lldb needs `libpython3.12.so.1.0`
   (system has 3.13, ABI-incompatible; no sudo). Got it from python-build-standalone:
   `curl -L <astral-sh/python-build-standalone …/cpython-3.12.13+20260610-x86_64-unknown-linux-gnu-install_only.tar.gz>`,
   extract `python/lib/libpython3.12.so.1.0` → run lldb with `LD_LIBRARY_PATH=<dir>
   PYTHONHOME=<extracted python>`. Then: `wasmtime run --env SWIFT_DETERMINISTIC_HASHING=1
   -g 127.0.0.1:1234 probe.wasm` (gdbstub; binds AFTER the ~80s JIT compile — poll the port) +
   `lldb -o 'process connect --plugin=wasm connect://127.0.0.1:1234' -o continue`. Gives FULL
   source-level stops: `apply_tmpl(this=0x3ed70e9c, options=2, body=0x02625488) at Subgraph.cpp`.
   GOTCHAS: lldb's `watchpoint set expression` + any expr eval CRASHES (ObjC-codegen bug in this
   lldb) — avoid expressions; use `memory read --format x --size 4` and raw packets. The gdbstub
   **does NOT support data watchpoints** (`process plugin packet send Z2,…` → `UNIMPLEMENTED`) — so
   NO hardware/data watchpoint; only breakpoints, single-step, memory/register read.

WHAT THE DEBUGGER REVEALED — it's a STACK corruption:
- Crash: `apply_tmpl this=0x3ed70e9c` (= the subgraph passed to `forEach`), `body=0x02625488`.
- A brute-force linear-memory scan (added in `apply_tmpl`) finds the value `0x3ed70e9c` at **40
  byte-stable addresses**, CLUSTERED around `0x2625440–0x262548c` = the **shadow-stack frame of the
  forEach/apply call chain** (right next to `body=0x02625488`).
- My registered item-slot scan (147 slots) + subgraph `_children` scan (152 subgraphs) match NONE of
  them → the victim is NOT a tracked heap field; it's the **subgraph pointer on the shadow stack**
  passed through `willRemove → forEach → AGSubgraphApplyC → apply_c → apply_tmpl`, overwritten by the
  interpolated animation CGFloat (`0x3ed70e9c` = float32 ≈ 0.42).
- Pinning all view-list items alive did NOT fix it (not premature-free); the inline-storage
  `vector<T,N>` migration (Vector.h:117 5-arg `realloc_vector`) + heap `vector<T,0>` are both CLEAN.
⇒ REVISED ROOT: a STACK buffer overflow / wrong-address write during the GeometryReader-animation
view update sprays an interpolated CGFloat over the subgraph pointer that `forEach` is about to use.

NEXT (now tractable thanks to determinism — no more heap-roulette):
- The gdbstub has no watchpoints, but determinism enables a reliable SOFTWARE watchpoint: poll the
  (now byte-stable) victim shadow-stack slot at a fine cadence (e.g. inside `value_set_internal` /
  the rule update) and trap on the write — OR `memory read` the `0x2625440–48c` frame at the crash
  to read the exact victim struct + offset, then breakpoint-step the prior frame.
- Repro recipe: build probe → `wasmtime run --env SWIFT_DETERMINISTIC_HASHING=1 -W max-wasm-stack=8388608
  .build/.../probe.wasm` (crashes MOVE 4, `0x3ed70eec`, deterministic).

### HW-watchpoint route (2026-06-23) — gdbstub has NO watchpoints; lldb now FIXED (see above); vector audit CLEAN
- wasmtime 45 has a wasm-level gdbstub (`-g PORT`, connect `lldb process connect --plugin=wasm
  connect://…`) — the CLEAN route (watchpoints on linear-memory addresses directly). BUT the only
  `lldb` here (swiftly 6.3.2) is broken: `libpython3.12.so.1.0` missing; system has 3.13 (ABI-crashes
  lldb); **no passwordless sudo** to `apt install lldb-19`/`libpython3.12`. So the gdbstub route is
  blocked in this environment. FIX when sudo is available: `apt install lldb-19` then
  `wasmtime run -g 127.0.0.1:1234 -D debug-info=y probe.wasm` + `lldb -o 'process connect --plugin=wasm
  connect://127.0.0.1:1234'` + watchpoint on the victim linear addr.
- GDB-host (gdb 16.3 works) is possible but heavy: must run past JIT registration to see wasm funcs,
  discover the linear-memory base (magic-search), translate wasm→host addrs, and the heap-roulette
  makes picking a STABLE victim address unreliable. Not pursued.
- Tool-free audit of the malloc-heap `vector<T,0,size_type>` (round-3 victim buffers: `_children`/
  `_parents`/`output_edges`) — **CLEAN**: `reserve_slow` (1.5× growth), `realloc_vector`
  (`malloc_good_size`=identity on wasm, `*size`=preferred), `push_back` (`reserve(_size+1)` then
  in-bounds placement) are all correct on wasm32. Not the overflow site.

STATUS: every static lead is clean; the bug is a genuine runtime within-update malloc-heap spray that
only an instruction-level memory watchpoint can pin — and that needs a working `lldb` (sudo) which
this env lacks. Best resumption: fix lldb → gdbstub watchpoint on symptom-#4's clobbered Subgraph
`_graph` (the deterministic anchor). Mitigation (drop the position `.animation`) keeps 2048 stable.

### (earlier this session) the spray is in the GeometryReader content-build view construction
The wasm32 buffer-overflow that sprays garbage over subgraph-pointer memory is in
`v.content(proxy)` — the re-evaluation that rebuilds the `_VariadicView.Tree<_LayoutRoot<
GeometryReaderLayout>, Content>` view hierarchy (`_makeViewList` / ViewList construction), triggered
every frame by the position animation. Everything around it is ruled out (value-read, offset attrs,
observers, item lifetime, Compute reuse, LayoutDescriptor). NEXT (needs a finer tool than
printf/trap-watchpoints, which are defeated by within-update + heap-sensitivity + spray):
- instrument `_VariadicView.Tree._makeViewList` / `_makeView` (the construction the content build
  runs) with checks bracketing each sub-step, OR
- a true wasm HW watchpoint (wasmtime GDB-JIT) on a victim slot located from a coredump.
WATCHPOINT INFRA (reusable; in /tmp + probe, NOT in the patches): `Subgraph.cpp` registry +
`wandr_scan_children` + ctor/dtor `wandr_sg_register`/`deregister`; `UpdateStack.cpp:260` check;
`ViewList.swift` + `DynamicContainer.swift` `@_extern(c)` register/deregister (+ `_wandrItemPin`);
`GeometryReader.swift` fine-grained `_wandr_watch_check`. Key gotcha: use `@_extern(c)` not
`@_silgen_name` for the C calls (silgen mislowers the ABI → `signature_mismatch`).
(Watchpoint infra lives in: Subgraph.cpp registry, UpdateStack.cpp:260 check, ViewList.swift +
DynamicContainer.swift register/deregister, GeometryReader.swift fine-grained checks.)

NEXT TOOL (to localize the wild write): static audits keep coming back clean → must OBSERVE the write
at runtime. A memory watchpoint (wasmtime GDB-JIT `--debug` + hw watchpoint on the victim slot) or a
value-keyed write-logger (log writes whose source bytes ≈ float 0.42 + destination; flag destinations
outside the writing node's buffer). Print-instrumentation relocates the victim, so prefer
watchpoint/value-keyed logging over more source edits. Remaining hypothesis: a swiftcc ABI
mislowering in the animation closure/listener path producing a wild funcptr/context (the recurring
wasm32 wall class) — but it must be caught at runtime, not by reading.

REPRO/DIAGNOSE recipe that worked (no GUI/device):
- Build probe (cmd below, §"Build/run the probe"); run
  `wasmtime run -W max-wasm-stack=8388608 -D debug-info=y .build/.../probe.wasm`.
- Bisect via the eleev `.transition`/`.animation` modifiers in `TileView.swift` / `TileBoardView.swift`.
- THE decisive detector: in `Graph::value_set_internal` (Graph.cpp), `#if __wasi__` log when
  `AttributeID(node_ptr).subgraph()->is_valid()` is false → prints the stale `type=DisplayList/Phase/
  ViewGeometry` writes on removed-tile subgraphs (the UAF, caught directly, no heap-layout sensitivity).
- To catch a bad subgraph pointer non-invasively: a `#if defined(__wasi__)` bounds-check on `this` at
  the top of `Subgraph::apply_tmpl` (Subgraph.cpp) + the `_ViewList_Subgraph.subgraph` raw bits at
  `ForEachState.eraseItem` (ForEach.swift) — but NOTE prints relocate the victim (heap-layout
  sensitive); the coredump/DWARF backtrace is the stable signal. (All such instrumentation reverted;
  /tmp trees clean.)
- MITIGATION available now (not a fix): removing the position `.animation` makes 2048 stable on wasm.

## 🔬 BREAKTHROUGH (2026-06-20 PM): the move-5-7 crash is TYPE CONFUSION, not DynamicContainer
The crash that kept "moving" (`pow` → `NativeBox<float vec>` → `Text.==` → `set_index`) is ONE bug:
a **wasm32 32/64-bit layout mismatch** that makes a `Float`/`CGFloat` value land where a pointer is
read. PROOF: the garbage "subgraph pointer" `0x3ed70eec` decodes as **IEEE float32 = 0.420036** (and
`0x3ed70edc` = 0.420035) — a CGFloat layout value (~0.42) read as a `Subgraph*`/class-ref pointer.
So it is NOT a bug in Apple-tested `DynamicContainer.items` (a plain Swift `[ItemInfo]` doesn't grow
garbage); the corruption comes from BELOW, in OUR untested wasm layer.

RULED OUT this session (decisive tests, don't re-chase):
- **`table::grow_region` dual-region memcpy** — pre-sized the Table to 64 MB so it never grows;
  STILL crashes at move 6, same `0x3ed70eec`. Not the table allocator.
- **attribute value copy** — `Graph::value_set_internal` uses `vw_assignWithCopy`/`vw_initializeWithCopy`
  (Swift value-witnesses, correct on wasm32). Not the copy.
- **`LayoutDescriptor`** — grep shows it's used ONLY by `AttributeType::compare_values[_partial]`,
  both of which I already memcmp-bypassed on wasm. Fully out of the live path.

PRIME SUSPECT = **`CGFloat` width** (the canonical 32/64 footgun: `Float`/4B on 32-bit, `Double`/8B
on 64-bit). float32-0.42 (not the half of a double) ⇒ a CGFloat is 4 bytes somewhere while a struct/
metadata/precompiled-module assumes 8 (or vice-versa) ⇒ every CGFloat-containing struct has shifted
offsets ⇒ a pointer field reads onto a CGFloat. The early `__POINTER_WIDTH__` directive
(`AGTargetConditionals.h:275`, makes wasm32 take `TARGET_RT_64_BIT==0`) only covers code that USES
`sizeof()`; CGFloat width / the precompiled Foundation module / any hardcoded 64-bit layout bypass it.
The commented-out `static_assert(sizeof(page)==0x18 / TreeElement==0x20 / HeapObject==0x10)` are
exactly such bypasses, silenced not fixed.

CONFIRMED (2026-06-20, `/tmp/cgprobe`, Foundation-only harness): `MemoryLayout<CGFloat>` = **4**
(align 4), CGPoint=8, CGRect=16, Double=8, Float=4, ptr=4. So CGFloat is `Float`/4B — CORRECT for a
32-bit target, and consistent across SDK Foundation + OpenSwiftUI source + Compute metadata reads.
⇒ **The simple "CGFloat is wrongly Double" hypothesis is REFUTED.** No global width mismatch.

But the type confusion still STANDS: the `0.42` is a `Float`/`CGFloat` (4B) read where a 4B **pointer**
is expected — a FIELD-OFFSET / layout disagreement, not a width bug (both are 4B, which hides it).
Refined culprit: OpenSwiftUI+Compute are reverse-engineered from Apple's AttributeGraph where
**CGFloat=8 (Double) AND ptr=8**. Any HARDCODED struct layout / `@frozen` offset / C++ struct that
mirrors an Apple geometry or node type encodes the 8-byte assumption → on wasm32 (CGFloat=4, ptr=4)
the offsets shift and a pointer slot lands on a Float. NOTE: ItemInfo itself has no bare CGFloat
(subgraph:ptr, uniqueId/viewCount:4, zIndex:Double) — so either `items[target]` points at a
NON-ItemInfo (a CGFloat-bearing struct e.g. CGRect/geometry) or the subgraph slot was overwritten by
a Float from an adjacent mislaid value. NEXT: instrument the exact write/read of the corrupted slot
(catch when a Float lands in a pointer field) — hunt hardcoded/Apple-8-byte layouts in the value/
node/geometry path, NOT a global CGFloat change. PROBE (stdout, `wasmtime run`, DWARF) is the tool.


## 🎮 MILESTONE (2026-06-20): eleev/swiftui-2048 is interactive on desktop
The actual eleev/swiftui-2048 `TileBoardView` + `GameLogic` runs in the wandr desktop GUI:
swipe (mouse drag) → `GameLogic.move` → slide/merge/spawn → board **re-renders on screen**
(verified: score climbs, `16` tiles form, tiles reposition). Demo = `repros/swift-canvas-spike`
`OpenSwiftUIDemo` target; clean component build via `build-openswiftui-demo.sh`.

### Two engine walls cracked today (both in `compute-wasm.patch`):
1. **`Subgraph.forEach` swiftcc closure-ABI wall** → SIGILL when `ForEach` erases a merged tile
   (`ForEachState.eraseItem → AGSubgraphRef.willRemove → forEach`). Fixed with the established
   `*C` pattern: `AGSubgraphApplyC` (C decl in `AGSubgraph.h` `#if __wasi__`) + `Subgraph::apply_c`
   + a shared `apply_tmpl` (Subgraph.{h,cpp}, AGSubgraph.cpp); Swift `forEach` routes through the
   C-imported `AGSubgraphApplyC` with a `@convention(c)` thunk (Subgraph.swift). NOT `@_silgen_name`
   (that mis-lowers to 6 i32 — import from the header like `AGGraphMutateAttributeC`).
2. **`LayoutDescriptor` value-comparison broken on wasm32** → `compare_values` /
   `compare_values_partial` / `AGCompareValues` returned "equal" for CHANGED view values, AND
   `compare_values_partial` dispatched a field's `Equatable` (e.g. `Text.==`) with a WRONG pointer
   → out-of-bounds fault (the SIGILL/SIGSEGV seen in `pow` / `Text.__derived_struct_equals` /
   `NativeBox<float vector[64]>` — whatever type sits at the bad offset). Root: 64-bit pointer/
   HeapObject-size assumptions in the field-layout walker (`LayoutDescriptor.cpp:562` static_assert
   commented for wasm). Worked around with conservative **`memcmp` on wasm** in ALL THREE entry
   points: `AttributeType::compare_values` + `compare_values_partial` (AttributeType.h) +
   `AGCompareValues` (AGComparison.cpp). This unblocked the display (children re-eval → tiles move)
   AND cleared the pow/Text/NativeBox crashes. Proper fix later = pointer-size audit of LayoutDescriptor.

### 🛠️ Diagnostic tool that works: the PROBE / stdout model (no GUI, no device)
`repros/openswiftui-wasm/probe` runs the 2048 game-logic + OpenSwiftUI render under **bare
`wasmtime`** (PrintSink, no wandr host, no wasi:canvas, no WSLg). Its `Main.main` drives moves via
the SAME path as the demo (`wandrApplyChange { tick += 1 }` + `wandrRender()`). On a trap wasmtime
prints the **full DWARF wasm backtrace** — this is how the comparison + the next crash were pinned
(WSLg weston is too flaky to watch the GUI; the device only logs frame #0). Build cmd is in the
probe's git history / the session; run: `wasmtime run -W max-wasm-stack=8388608 .build/.../probe.wasm`.

### ✅ Fixed: `willRemove` use-after-free (in `openswiftui-phase1-wip.patch`)
`Subgraph.willRemove()` (AttributeGraphAdditions.swift) now **collects** removable attributes
during `forEach`, THEN processes them — so the callbacks (which free subgraphs) don't run while
the fork's `apply_tmpl` iterates the subgraph tree live. Cleared the `is_valid()` crash.

### 🔲 CURRENT frontier (probe survives ~5-7 moves, display correct, then SIGSEGV):
Backtrace: `Graph::UpdateStack::update → DynamicContainerInfo.updateValue → AGSubgraphSetIndex →
Subgraph::set_index()`. Drilled in with prints (DynamicContainer.swift:447 `info.items[target].
subgraph.index = ...`): the printed `sgRaw` values are SANE (~41M, in bounds) for most iterations,
and a freshly CREATEd item's subgraph is valid too — **the crash is the iteration right after the
items array GROWS** (e.g. count 2→3 when a tile spawns): `info.items[target]` returns a **garbage
class ref** (value ~4 GB, VARIES run-to-run → uninitialized, not a fixed offset, not a freed ptr).
`info.items` is a plain Swift `[ItemInfo]` (DynamicContainer.swift:34), yet an element is garbage —
so a garbage ref enters the array during the grow/REORDER. Suspects: the items/displayMap permutation
(`info.items[validCount..<inusedCount] = slice` ~L368, the `displayMap` insertionSort/reorder ~L421),
the `target = displayMap[...]` mapping, OR `ContiguousArray<class>` reorder on wasm32. NEXT: print
`info.items` element raws before/after the slice-reorder to catch where the garbage ref is written;
the PROBE (stdout model, `wasmtime run`) reproduces it in seconds with full DWARF backtraces — that's
the tool. (Diagnostic prints were reverted; the probe's `Main.main` move-loop driver is left in place.)

### NOTE — this is a SERIES of wasm32 AttributeGraph bugs
The dynamic-view-update + subgraph-removal path on wasm32 has multiple pointer-size/offset bugs
(LayoutDescriptor comparisons → fixed via memcmp; willRemove UAF → fixed; now set_index). Each fix
exposes the next. The game RENDERS + PLAYS with correct display (forEach + comparison walls solved);
the remaining crashes are this removal/index series. A pointer-size audit of the AttributeGraph C++
(Subgraph layout, children/index iteration) on wasm32 is the durable fix vs whack-a-mole.

### Reactor reactivity (in `openswiftui-phase1-wip.patch` WandrApp.swift + the demo `main.swift`):
- `@ObservedObject`/OpenCombine **crashes on wasm** (FilterProducer / ObservableObjectPublisher
  generic witnesses; its C++ UnfairLock also corrupts the reactor runtime). So GameLogic has
  OpenCombine **stripped** (plain class; no ObservableObject/@Published/objectWillChange) and the
  REACTOR drives reactivity: `@State tick` in ContentView, bumped in `on_pointer` via new SPI
  `wandrApplyChange { move; tick += 1 }` (mutate inside an Update transaction), then `on_frame`
  calls `wandrRender()` (re-run graph) + `wandrRedraw()` (paint; the renderer's seed guard skips
  its own draw). `sharedGame` constructed explicitly in `on_frame` (not a lazy global — lazy-global
  init inside the graph eval left `tileMatrix` nil). `.id(tick)` on TileBoardView.

### 🔲 REMAINING device blocker: `pow` SIGILL (aarch64-AOT only)
On device, after ~3-4 moves: `SIGILL function[170857]::pow+1320`. Works fine on x86 JIT/desktop
(no crash, board plays through) → it's an **aarch64 cross-AOT `pow` codegen issue**, triggered by
the animations that now run (the `memcmp` fix enabled re-renders → eleev's `.modalSpring`
transitions). Next: disable animations (so the animation `pow` path isn't hit) OR fix aarch64-AOT
`pow`. Desktop is unaffected — fully playable there (use `WINIT_UNIX_BACKEND=x11`; WSLg weston is
flaky). Adapter: `tools/wasi-adapters/wasi_snapshot_preview1.reactor-45.0.0.wasm` (official; the
skiko one is ~identical).

---

# OpenSwiftUI on wasm — resume point (2026-06-19, ✅ FIRST ON-DEVICE PIXEL)

## 🎉 MILESTONE (2026-06-19): real SwiftUI renders ON THE PIXEL 2 XL
`VStack { Color.red; Color.blue }` (OpenSwiftUI, phases 1–4) renders on the device —
red top / blue bottom, chrome intact, stable past 35 s (screenshot verified). End-to-end:
OpenSwiftUI AttributeGraph → `DisplayList` → `WandrDrawSink` → CGContext → wasi:canvas →
Skia/EGL. App = `apps`-installed `wandr.swiftui.demo` (built from `repros/swift-canvas-spike`
`OpenSwiftUIDemo` target).

### ✅ TEXT now renders too (2026-06-19) — delegated to the host, no guest text engine
`Text("…").font(.system(size:64,weight:.bold)).foregroundColor(.yellow)` renders correctly on
device (size + color honored). OpenSwiftUI's off-Apple text path is stubbed (ResolvedStyledText
storage nil, ShapeStyle/glyph rendering unimplemented), so instead of building a text engine we
**emit a `.content(.text)` leaf and let the host (wasi:canvas paragraph / Skia) shape + draw it**:
- `StyledTextContentView` routes `_makeView` through `RendererLeafView`/`LeafViewLayout` on wasm
  (bypassing the stubbed ShapeStyle path), carrying `wasmPlainString` (from `Text._localizationInfo`
  — safe; `AnyTextStorage.debugDescription` faults off-Apple), `wasmFontSize`, `wasmColor`.
- `TextChildQuery.value` resolves the point size (`Font.resolveTraits(in:).pointSize`) + foreground
  (`environment.foregroundColor?.resolve(in:)`) from the environment.
- `Font.scaleFactor` got an off-Apple `#else` (identity at default Dynamic Type) so **system fonts
  resolve without CoreText** (CoreText is unavailable on wasm; semantic styles need this).
- `WandrDrawSink.drawText` + walker `.content(.text)` case → `CGContext.drawString` → wasi:canvas.
- GOTCHA: the walker draws children in order, so the text's reserved band (size × 1.35) must match
  the drawn size or the next sibling paints over it.
Remaining polish: host-measure `sizeThatFits` (currently `chars × size × 0.6` estimate) for exact
width; semantic text-style sizes assume the default Dynamic Type category.

### What unblocked on-device = CROSS-AOT (not footprint/crypto/threads/GC)
The device can't fit the AOT compile of this 172k-function component: full-parallel
cranelift peaks **~2 GB RSS** (thread count barely matters — serial is still ~1.96 GB; it's
the module, not parallelism) and the device process OOMs (~2.3 GB free, ~290 MB headroom).
Dropping swift-crypto/BoringSSL saved only ~1.3 MB (it was mostly DWARF). The fix: compile
the **aarch64 cwasm on the PC** and push it — the device just deserializes (`loader: cache
fresh`, no recompile). Target is part of wasmtime's `precompile_compatibility_hash`, so the
`aarch64-linux-android` triple matched the device's engine hash exactly. **This generalizes
to any large guest that can't AOT on-device.**

### CROSS-AOT deploy recipe (reproducible)
```bash
# 1. Build the desktop host WITH all cranelift ISA backends (one-time):
cd runtime/wandr-host && cargo build --release --target x86_64-unknown-linux-gnu --features cross-aot
# 2. Build the guest component (repros/swift-canvas-spike), strip, drop into a wandrpkg:
cd repros/swift-canvas-spike && ./build-openswiftui-demo.sh    # -> .build/.../OpenSwiftUIDemo.wasm
wasm-tools strip <core>.wasm -o x.wasm && wasm-tools strip --delete '^name$' x.wasm -o x2.wasm
wasm-tools component new x2.wasm --adapt wasi_snapshot_preview1=<adapter> -o <pkg>/components/ui.wasm
# 3. Cross-AOT FOR the device on the PC (WANDR_AOT_TARGET sets wasmtime's compile target):
WANDR_AOT_TARGET=aarch64-linux-android WANDR_APPS_ROOT=/tmp/stage \
  <x86_64-host> --install <pkg>          # -> /tmp/stage/apps/<id>/<ver>/{cache/ui.cwasm, cache-key.toml, ...}
# 4. Push the staged install dir to the device apps root + launch:
adb shell "su -c 'rm -rf $APPS/<id>'"; adb push /tmp/stage/apps/<id>/<ver> $APPS/<id>/<ver>
adb shell "su -c 'WANDR_APPS_ROOT=... wandr-arbiter launch <id>'"
```
Host support: `make_config()` (shared engine config) + `WANDR_AOT_TARGET` override in
`install_wandrpkg` (lib.rs) + the `cross-aot` Cargo feature (`wasmtime/all-arch`).

### GOTCHA — `AppGraph.shared` is set once per process
`renderWandrAppOnce` sets the once-only `AppGraph.shared`; calling it twice fatalErrors
("may only be set once") → SIGILL. The guest must build the graph EXACTLY once and never
rebuild (e.g. don't rebuild on resize). Cost us a ~10 s-in crash until fixed.

## ▶ MULTI-CHILD WALL CLEARED — VStack renders 2 fills on wasm
WORKING on wasm32-wasip1: primitive + custom Views, **@State (reactive)**, **Text
(construction)**, single-type conformance, **DisplayList rendered**, AND NOW **multi-child
`VStack`/`TupleView`**. `VStack { Color.red; Color.blue }` →
```
surface: 640.0x480.0
display-list-version: 4
rendered:
  - fill x:0.0 y:0.0 w:640.0 h:236.0 #FF3B30FF      # Color.red, top half
  - fill x:0.0 y:244.0 w:640.0 h:236.0 #007AFFFF     # Color.blue, bottom (8pt VStack gap)
```
exit 0, **deterministic across 5 runs** (the nondeterminism was the bug's fingerprint — gone).
`VStack { Color.red.opacity(count>5 ?…); Text("count \(count)"); Color.blue }` with `@State
count=7` also runs to exit 0 → `fill … #FF3B3040` (alpha 0x40 = 0.25; @State flows through the
reactive graph on the multi-child path). Text emits no `fill` (Text layout/render are
`unimplemented` stubs off-Apple — real glyphs = phase-4 CGContext drawer).

### THE ROOT CAUSE (it was NOT memory corruption / TupleView witness tables)
The prior "memory corruption, can't localize" framing was WRONG — that was the *symptom* of a
swiftcc closure-with-arg/return signature mismatch seen through print-probing. Diagnosed
**non-invasively** with `wasmtime run -D debug-info=y -D coredump=… -D max-backtrace=N` (no
guest source change → immune to "instrumentation moves the crash"). Frame 0 of the
DWARF-symbolized backtrace was literally **`signature_mismatch:AGGraphReadCachedAttribute`**,
reached via `Compute.Rule._cachedValue` (Rule.swift:104) ← `SizeAndSpacingContext.dynamicMember`
(PlacementContext.swift:47) ← `ViewLayoutEngine.layout` — i.e. the **layout engine** reading a
cached environment attribute. Multi-child views run real layout; single-child (full-surface
Color) didn't hit `cachedValue`. `AGGraphReadCachedAttribute` was exactly the symbol the
"bounded swiftcc set — would TRAP if hit" list predicted. (`OAGTupleWithBuffer`, the witness
tables, the `unsafeBitCast` existential write — all RULED OUT.)

### THE FIX (in `compute-wasm.patch`, validated in the 4s harness then the probe)
Established plain-C `*C`-variant pattern (mirrors `AGGraphInternAttributeTypeC`):
- `extern "C" AGGraphReadCachedAttributeC` in `ComputeCxx/Graph/AGGraph.cpp` + header decl in
  `AGGraph.h` (`#if defined(__wasi__)`) — a separate symbol (the RESUME-proven rule: a
  `@convention(c)` thunk to the *original* symbol still traps; only a separate `*C` symbol works).
- GOTCHA: on wasm `AG_SWIFT_CC(swift)`/`AG_SWIFT_CONTEXT` are NOT empty → `ClosureFunctionCI`
  is hard-`swiftcall` and can't be built from the plain-C thunk. So **`Subgraph::cache_fetch`
  was templatized** (`cache_fetch_tmpl<Getter>`) so a plain-C `Subgraph::PlainTypeIDGetter`
  (calls `fn(graph, ctx)` directly, like `intern_type_c`) threads through alongside the
  swiftcall `ClosureFunctionCI`. `read_cached_attribute` in AGGraph.cpp templatized likewise.
- Swift `Rule._cachedValue` (`Compute/Attribute/Rule/Rule.swift`) routes `#if arch(wasm32)`
  through `AGGraphReadCachedAttributeC` with a heap-boxed closure (`_CachedAttrBox`) + a
  non-capturing `@convention(c)` trampoline. The closure is SYNCHRONOUS, but `ClosureFunctionCI`
  `swift_retain`s the context → must pass a real heap object (box), not a stack pointer;
  `withoutActuallyEscaping(makeTypeID)` + `withExtendedLifetime(box)` keep it valid+alive.

### NEXT
- **✅ PHASE 4a DONE — DisplayList wired into a drawing sink (desktop-verified).** Added a
  `.wandr` renderer mirroring the `.stdout` trio (in `openswiftui-phase1-wip.patch`):
  `RendererConfiguration.swift` (`case wandr(WandrOptions)` + factory + `public WandrOptions`
  + `public protocol WandrDrawSink`), `WandrDisplayListRenderer.swift` (the recursive
  `DisplayList`→sink walker, mirror of `StdoutRenderCommandVisitor`: resolves color / solid
  shape / opacity / affine transform; clip/text/image/mask = TODO), `WandrRendererHost.swift`
  (`package`, mirror of `StdoutRendererHost`, configures `.wandr`), `WandrApp.swift`
  (`@_spi(WandrRenderer) renderWandrAppOnce`, retains the host to dodge the `Subgraph.forEach`
  teardown wall). Probe now drives it with a `PrintSink` →
  `wandr fillRect x:0 y:0 w:640 h:236 rgba(1.000,0.231,0.188,1.000)` (red) +
  `… y:244 … rgba(0.000,0.478,1.000,1.000)` (blue), exit 0 — the real `DisplayList` resolves
  to the exact fills (frames + sRGB) a CGContext would draw. `WandrDrawSink` is plain scalars
  (no CoreGraphics types) so the guest stays decoupled. (Harmless stderr `DecodingError` =
  AppGraph `archiveJSON` no-op stub; rendering unaffected.)
- **PHASE 4b NEXT — implement `WandrDrawSink` over the spike's wasi:canvas `CGContext` in a
  real guest, deploy, visual check.** The sink body is `cg.setFillColor(CGColor(red,green,blue,
  alpha:opacity)); cg.fill(CGRect(x,y,w,h))` — exactly `repros/swift-canvas-spike`'s
  `DisplayListRenderer`/`spike.swift` pattern (`on_frame` acquires the wasi:canvas context →
  builds `CGContext` → draws). Make a guest app depending on OpenSwiftUI + the spike's
  `OpenCoreGraphics`(over wasi:canvas) + `CSwiftSpike`; its `on_frame` calls `renderWandrAppOnce`
  ONCE (host persists) — but per-frame re-render needs a `render(into:)`-on-existing-host API
  (the one-shot host renders once then returns `.infinity`). Then component-package + deploy to
  Pixel 2 XL; the red/blue split is the first on-device SwiftUI pixel (visual check = WITH USER,
  per [[feedback_visual_verification]]).
- **Then grow the sink + walker:** add `fillPath`/`pushClip`/`pushTransform`/`drawText` to
  `WandrDrawSink` + the matching `DisplayList.Effect`/`Content` cases in
  `WandrDisplayListRenderer` (clip/mask/blend/filter currently recurse unmodified; text/image
  hit the `default: break`). Real glyphs also need Text's unimplemented off-Apple layout stubs
  (`Text+View.swift sizeThatFits/spacing/explicitAlignment`, `ResolvedText`, `ShapeStyleRendering`).
- **Remaining bounded swiftcc walls** (still in the linker `function signature mismatch`
  warnings; will TRAP only as richer views reach them — same `*C` fix each, validate in the 4s
  harness `repros/compute-wasm/computerun` first): `AGGraphSearch`, `AGTupleWithBuffer`,
  `AGGraphWithUpdate`, `AGTypeApplyEnumData`/`AGTypeApplyMutableEnumData`, `AGSubgraphAddObserver`,
  `AGSubgraphApply`.
- Build/run + diagnosis recipe below. Patches match `/tmp` (openswiftui-phase1-wip + compute-wasm
  + oag-fork); `compute-wasm.patch` base = `efb754b` (HEAD of harryzz/Compute `wasm32-wasip1-osp`).

## 🎉 PHASE 3 FIRST PIXEL (2026-06-19): OpenSwiftUI renders a DisplayList on wasm
`WindowGroup { Color.red }` → built + run under wasmtime → **clean exit 0**:
```
OpenSwiftUI backend: stdout
surface: 640.0x480.0
display-list-version: 3
rendered:
  - fill x:0.0 y:0.0 w:640.0 h:480.0 #FF3B30FF      # Color.red, full surface
```
A real SwiftUI view, laid out by the AttributeGraph (Compute) engine, emitted as an
OpenSwiftUI `DisplayList`, end-to-end on wasm32-wasip1. This is exactly the input the
device-verified Option-B drawer (`repros/swift-canvas-spike/.../DisplayListRenderer.swift`)
turns into CGContext → wasi:canvas → Skia/EGL. **Phase 3's core unknown is resolved.**

The fix that delivered it: `exit(0)` immediately after `host.renderOnce()` in
`StdoutApp.swift` (`#if os(WASI)`) — the render always completed; the only trap was in
TEARDOWN (`GraphHost.deinit → Subgraph.forEach`, an arg-closure swiftcall wall), and the
abort was happening before stdout flushed. Exiting before the closure's locals deinit
sidesteps that wall AND flushes stdout. (a)+(b) were the same issue.

### ✅ @State + Text WORK (2026-06-19): onUpdate/onInvalidation wired as stored plain-C callbacks
`WindowGroup { ContentView() }` with `@State` renders on wasm, clean exit 0:
- `ContentView { @State count=7; Text("count \(count)") }` → renders (empty `rendered:`
  because the stdout renderer emits only fills, not `.text` — real glyphs = phase-4 drawer).
- **Visible @State proof** (this exact view HUNG before the fix):
  `Color.red.opacity(count > 5 ? 0.25 : 1.0)` with count=7 →
  `fill … #FF3B3040` (red, alpha 0x40 = 0.25). @State is stored, read in `body`, and its
  value flows through the reactive graph into the DisplayList. Deterministic (re-ran clean).
- THE FIX (in `compute-wasm.patch`): `onUpdate`/`onInvalidation` were no-op'd; now wired as
  STORED plain-C callbacks. C++ `Context` gets `_update_callback_c`/`_invalidation_callback_c`
  (plain fn+ctx) + `set_*_callback_c`, invoked plain-C in `call_update`/`call_invalidation`;
  `extern "C" AGGraphSet{Update,Invalidation}CallbackC` entry points; Swift `Graph.swift`
  boxes the closure (`_GraphUpdateBox`/`_GraphInvalidationBox`) + passes a non-capturing
  `@convention(c)` trampoline + `Unmanaged.passRetained` box ptr (lives for graph lifetime).
  Mirrors the fork's existing `_UpdateBox`/`AGRetainClosureC` rule-update pattern. Gotcha:
  the `*C` context param is non-optional (`const void*` w/o `_Nullable`) → trampolines take
  `UnsafeRawPointer`, not `UnsafeRawPointer?`.
- Earlier "custom View hangs" was a stale build; the real hang was @State, now fixed.

### 🔶 VStack/TupleView (2026-06-19): conformance wall cleared; new "undefined element" wall
- ✅ **`swift_conformsToProtocol` C-shim DONE** (in `openswiftui-phase1-wip.patch`): it's
  `C_CC` per Swift `RuntimeFunctions.def:1859` (NOT swiftcall — specialist was wrong), so
  the `@_silgen_name` call mislowered. Added `_OpenSwiftUI_conformsToProtocol` plain-C
  wrapper in `OpenSwiftUI_SPI/Util/ProtocolDescriptor.{h,c}` (forwards to the C_CC runtime
  symbol) + `TypeConformance.swift` `#if os(WASI)` routes through it. The conformance trap
  is GONE.
- ⛔ **NEW WALL (diagnosed): `wasm trap: undefined element: out of bounds table access`**
  in the **`DebugReplaceableView` type-eraser dispatch**. Hit by `VStack { Color.red; Color.blue }`
  (no @State/Text needed). Consistent trap (nondeterministic trap-vs-hash → a WILD/uninitialized
  function pointer: an OOB table index traps, a valid-but-wrong index hangs).
  Path (verified backtrace): `ViewGraph.updateOutputs → Subgraph::update →
  StatefulRule._update → DynamicViewContainer.updateValue` (`View/DynamicView/DynamicView.swift:73`)
  → `view.makeChildView()` where `view: V = DebugReplaceableView` →
  `DebugReplaceableView.makeChildView` (`DebugReplaceableView.swift:94`) →
  `storage.makeChildView` (virtual dispatch into `DebugReplaceableViewStorage<Content>`) →
  the type-erased indirect call to `Content`'s make-function is OOB on wasm (Content =
  `TupleView<(Color,Color)>`). `@_typeEraser(DebugReplaceableView)` is gated on
  `OPENSWIFTUI_SUPPORT_2025_API && compiler(>=6.2)` (NOT DEBUG) — so a release build won't
  avoid it. Single views go through DebugReplaceableView fine; only multi-child OOBs.
  - WORKAROUND TRIED & FAILED (reverted): disabling `@_typeEraser(DebugReplaceableView)` on
    wasm (View.swift) only changed the symptom (OOB trap → hang) — because `AnyView` ALSO
    routes through `makeDynamicView` → `DynamicViewContainer` (AnyView.swift:57). So the eraser
    CHOICE isn't the cause.
  - REAL ROOT (localized): the failure is the per-element **runtime-conformance witness
    dispatch** for `TupleView` content, NOT the eraser. `TupleView._makeView`
    (`View/TupleView.swift`) builds each child via `TypeConformance<ViewDescriptor>.visitType`
    (`View/View.swift:201`) → `unsafeExistentialMetatype` does
    `unsafeBitCast(storage, to: (any View.Type).self)` packing the witness table returned by
    `swift_conformsToProtocol` into an existential metatype, then calls `_makeView` through that
    witness. On wasm a witness-table function-pointer entry resolves to an OOB `call_indirect`
    (nondeterministic trap/hang = a wild funcref index). Single-view Content works (it doesn't
    take this runtime-conformance-per-element path); only TupleView does.
  - 🔑 **KEY FINDING (hypothesis-(a) instrumentation attempt): it's MEMORY CORRUPTION, not a
    static missing witness.** Adding `_wasmDiag` (unbuffered `write(2)`) prints at
    `DynamicViewContainer.updateValue` / `tupleDescription` / `visitType` produced ZERO output
    and made the binary hang IMMEDIATELY (before even the usual "HitTestBindingModifier
    unimplemented" warning). I.e. **the failure point moves dramatically with any code change**
    — the classic signature of a wild pointer from corrupted memory. Print-instrumentation
    CANNOT localize it (adding a probe relocates the crash). The "undefined element" trap vs
    hang nondeterminism is the same story. So the next approach must NOT be print-probing:
    use memory bisection / a wasm memory sanitizer, or find the upstream ABI mismatch that
    corrupts memory only on the multi-child (TupleView) path (single-child is clean). Prime
    suspect: tuple metadata reflection (`Compute TupleType.type(at:)`/`AGTupleWithBuffer`) or
    the `unsafeBitCast(storage, to: (any View.Type).self)` existential write, doing a
    wrong-sized/wrong-offset read/write on wasm32. (Instrumentation reverted; tree clean.)
  - HYPOTHESES for the real fix (need Swift-runtime-on-wasm investigation):
    (a) the witness table from `swift_conformsToProtocol` for tuple element types may be an
        un-instantiated generic pattern on wasm (entries are bad) — may need
        `swift_conformsToProtocolCommon`/a different lookup, or instantiating the witness;
    (b) the `unsafeBitCast(storage, to: (any View.Type).self)` existential-metatype layout or
        the funcref representation differs on wasm (witness fn pointers are table indices);
    (c) `ViewDescriptor.tupleDescription`/`TupleType` element enumeration yields wrong
        conformances on wasm. Next: instrument `TypeConformance.visitType`/`tupleDescription`
        to print the element type + witness, and check whether `_makeView` via the witness is
        the exact OOB call.
  - `VStack+@State+Text` HANGS = the valid-but-wrong-funcref variant of the same wild pointer;
    same root, clears together.
- Bounded Compute swiftcc set still pending IF hit (would TRAP, not seen yet):
  `AGGraphReadCachedAttribute`, `AGGraphSearch`, `AGTupleWithBuffer`, `AGTypeApplyEnumData`/
  `MutableEnumData` (validate each in the 4s harness `repros/compute-wasm/computerun`).

### NEXT (other)
- `Subgraph.forEach`/`AGSubgraphApply` `*C` variant for clean teardown (currently sidestepped
  by `exit(0)` in StdoutApp before deinit).
- PHASE 4: wire the real `DisplayList` into the Option-B CGContext drawer → device. The
  stdout renderer not emitting `.text` is fine — the CGContext drawer handles `.text`.

### (historical) reading `@State` HANGS — root = onUpdate/onInvalidation no-op (NOW FIXED above)
Progress toward Text+@State (all verified by running the probe):
- Custom `View` with a reactive `body` **renders fine** (the earlier "custom View hangs"
  was a STALE incremental build — confirmed `ContentView { Color.red }` renders 3/3).
- Cleared two OpenSwiftUI off-Apple stubs (in `openswiftui-phase1-wip.patch`):
  - `_GraphInputs.defaultInterfaceIdiom` (`InterfaceIdiomPredicate.swift:36`) was
    `_openSwiftUIUnimplementedFailure()` on non-Apple → default to `.phone` (wandr is a
    phone-class device). Needed by `Text.makeCommonAttributes`.
  - `Text` localization hit `Bundle.main`, which CRASHES in Foundation's lazy `_mainBundle`
    init on wasm. `Text+Localized.swift` (resolve + resolvesToEmpty) now `#if os(WASI)`
    localize only if an explicit bundle was passed, else use the key literally (the
    documented no-table fallback). No more Bundle.main on wasm.
- **THE WALL: reading `@State` hangs.** `ContentView { @State count; Color.red.opacity(count>0 ?…) }`
  → infinite loop (exit 124). Custom View *without* @State renders; adding @State + reading
  it spins. The spin is NOT in `Update.dispatchActions` nor the render-driver loop (bounded
  counters never tripped — instrumentation since removed). It's in the reactive
  update/invalidation machinery → almost certainly the **`onUpdate`/`onInvalidation` no-op
  shortcut** (Compute `Graph.swift`, `#if arch(wasm32)`): @State's dependency invalidation
  can't propagate/settle, so the graph never reaches steady state.
- **FIX (next): properly wire `onUpdate` AND `onInvalidation` as STORED plain-C callbacks.**
  Unlike the synchronous callbacks (forEachField/mutateBody), these are stored and invoked
  later by C++, so box the Swift closure (heap object) + pass a non-capturing
  `@convention(c)` trampoline + retain/release the box. The Compute fork ALREADY has this
  exact template: `_UpdateBox` + `AGRetainClosureC`/`AGReleaseClosure` in
  `Attribute/AttributeType.swift` (used for the rule `_update` callback). Mirror it for
  `AGGraphSetUpdateCallback`/`AGGraphSetInvalidationCallback` (need C++ `*C` setters that
  store a plain-C fn+ctx and invoke plain-C in `Context::call_update`/invalidation).
- Text itself: construction + resolution now work; whether Text *layout* (fonts/paragraph,
  a `StatefulRule`) has further walls is UNKNOWN — @State hangs before we get there. Note
  the stdout renderer doesn't emit `.text` items anyway (only fills); real glyphs come via
  the phase-4 CGContext drawer.

### ⛔ (earlier framing) custom `View` with a reactive `body` — RESOLVED (was a stale build)
Isolated: `Color.red` DIRECTLY in `WindowGroup` renders ✅; wrapping it in
`struct ContentView: View { var body: some View { Color.red } }` → **infinite loop**
(wasmtime exit 124, no stderr). Independent of `@State`/`Text` (both also hang, same cause).
- The render driver's outer `repeat…while true` is bounded (breaks in ≤2 iters); the spin
  is INSIDE the update machinery. Prime suspect: `Update.dispatchActions()`
  (`Data/Update.swift:191`) — `repeat { … } while !Update.actions.isEmpty` with **no
  iteration bound**; if dispatched actions keep re-enqueuing, it never drains.
- Likely root: the `onUpdate`/`onInvalidation` **no-op shortcut** (Compute `Graph.swift`,
  `#if arch(wasm32)`) breaks the update-completion signal for reactive bodies; primitive
  views don't exercise it, custom views do. This is the phase-3 follow-up previously
  flagged as "owed".
- DIAGNOSE NEXT: add a bounded counter + `preconditionFailure` in `dispatchActions`
  (and/or `ViewGraph` transaction loop) to convert the hang into a backtrace and confirm
  the spinner + the re-enqueuing action. Then FIX: properly wire `onUpdate` as a plain-C
  callback variant (like `attribute_modify_c`/`AGGraphMutateAttributeC` — it's a STORED
  callback, so box the closure: see the fork's existing `_UpdateBox` + `AGRetainClosureC`
  pattern in compute-wasm) and/or drain `_wasmDrainMainRunLoop()` so transactions settle.
- Minimal repro is the current `probe/Sources/Probe/ProbeApp.swift` (custom View, no State).

### NEXT (for richer UIs — Text / VStack / @State), all bounded + validatable in the 4s harness
- `Subgraph.forEach`/`AGSubgraphApply` `*C` variant — for clean teardown + views that
  call forEach mid-render. (engine `Subgraph::apply_c`, like `attribute_modify_c`.)
- `swift_conformsToProtocol` C-shim in `OpenSwiftUI_SPI` — unblocks `VStack`/`TupleView`.
- Bounded Compute set: `AGGraphReadCachedAttribute`, `AGGraphSearch`, `AGTupleWithBuffer`,
  `AGTypeApplyEnumData`/`MutableEnumData`.
Then PHASE 4: wire the real `DisplayList` into the Option-B CGContext drawer → device.

## 🔭 (earlier) PHASE 3: the OpenSwiftUI render pipeline RUNS on wasm
De-risk probe `repros/openswiftui-wasm/probe` (a `@main App { VStack { Color.red; Color.blue } }`)
builds to `probe.wasm` (~138MB debug) and **executes under wasmtime**: `App.main →
runStdoutApp → AppGraph → GraphHost → ViewGraph.instantiateOutputs → render → View
construction`. The AttributeGraph/Compute reactive engine genuinely runs in wasm. The
remaining work is a mechanical grind of ABI walls (below).

### Build/run the probe (NOTE: ANY_ATTRIBUTE_FIX=0 is now REQUIRED)
```bash
cd repros/openswiftui-wasm/probe
OPENSWIFTUI_ANY_ATTRIBUTE_FIX=0 ANY_ATTRIBUTE_FIX=0 \
OPENSWIFTUI_USE_LOCAL_DEPS=1 OPENATTRIBUTEGRAPH_OPENATTRIBUTESHIMS_COMPUTE=1 \
OPENATTRIBUTEGRAPH_USE_LOCAL_DEPS=1 \
OPENRENDERBOX_LIB_SWIFT_PATH=/tmp/oag-fork/Sources/SwiftCorelibs/include \
swift build --product probe --swift-sdk swift-6.3.2-RELEASE_wasm \
  -Xcc -I/tmp/oag-shims -Xcc -fno-exceptions -Xcc -DSWIFT_INLINE_NAMESPACE=__runtime \
  -Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN -Xcc -D_WASI_EMULATED_PROCESS_CLOCKS
wasmtime run .build/wasm32-unknown-wasip1/debug/probe.wasm   # prints the DisplayList when done
```

### Phase 3 walls cleared (in the patches)
1. **Link-time CFRunLoop/Timer undefined** — libs compiled but linking the exe surfaced
   Foundation symbols absent on wasm. Guarded CF code in `RunLoopUtils.swift`
   (addObserver/runAllowingEarlyExit/CFRunLoopMode) + a `Timer` shim shadowing
   `Foundation.Timer` in `WasmThreadingShim.swift` (its impl pulls CFRunLoopTimer*).
2. **`OPENSWIFTUI_ANY_ATTRIBUTE_FIX` → `preconditionFailure("#39")` stubs** — disabled the
   fix (`ANY_ATTRIBUTE_FIX=0`) to bind the REAL Compute `AnyAttribute`/`onInvalidation`/
   `Subgraph.*`; needed one `package import` fix (`View_Indirect.swift`).
3. **swiftcc closure ABI mismatch (the main grind).** Swift closures passed to
   `AG_SWIFT_CC(swift)` C fn-pointer params mislower on wasm → `signature_mismatch` trap.
   KEY: zero-arg `()->Void` closures lower fine (e.g. `AGSubgraphApply` works); closures
   **with args or a return value** break. Each broken one needs a plain-C `*C` variant:
   C++ engine `_c` method (if the closure is invoked inside the engine) + `extern "C"`
   wrapper + header decl (`#if defined(__wasi__)`) + Swift `#if arch(wasm32)` routing with a
   non-capturing `@convention(c)` thunk + boxed/by-pointer context. Template = the fork's
   pre-existing `AGGraphInternAttributeTypeC` (synchronous: `withoutActuallyEscaping` +
   `withUnsafePointer`). Cleared so far: `onUpdate`/`onInvalidation` (guarded no-op),
   `withMainThreadHandler` (run body directly), `forEachField` (`AGTypeApplyFields/2C`),
   `AGGraphMutateAttribute` (`attribute_modify_c`). Gotcha: C params without `_Nullable`
   import as NON-optional in the `@convention(c)` thunk (don't force-unwrap).

### 🧭 STRATEGY (validated 2026-06-19 — two agents + an empirical test)
Root cause (confirmed): `swiftcall` carries closure context/error in *register* slots
absent on wasm; clang (C++ engine) and swiftc disagree on the `call_indirect`
function-table type → `signature_mismatch`. So zero-arg `()->Void` lower fine; closures
**with args/return** break. **No compiler flag fixes it; 6.3.2 is the latest SDK; the
upstream auto-thunk fix was reverted → a toolchain bump will NOT help.**
- TESTED & REJECTED the proposed "one header edit" general fix: flipping
  `AG_SWIFT_CC`/`AG_SWIFT_CONTEXT` to plain-C on wasm in `AGBase.h`. With a raw Swift
  closure → still traps (a Swift closure's own fn is swiftcc). With a `@convention(c)`
  thunk to the **original** symbol → STILL traps. Only a **separate `*C` symbol** +
  `@convention(c)` thunk works (= the per-function pattern already in use). Reverted it.
- REAL process wins (kill the "cycling"): (1) validate each Compute `*C` shape in the
  **seconds-fast** harness `repros/compute-wasm/computerun` (~4s rebuild) BEFORE the
  ~15-min probe rebuild; (2) the remaining surface is BOUNDED — ~5 Compute callbacks +
  exactly ONE Swift-runtime symbol (`swift_conformsToProtocol`); (3) target the smallest
  view first.

### 🎉 MILESTONE (2026-06-19): single-`Color.red` probe — render pipeline RUNS TO COMPLETION
Shrinking the probe to `WindowGroup { Color.red }` gets PAST View construction and the
whole render — the only trap now is in **TEARDOWN**: `runStdoutApp` closure ends →
`StdoutRendererHost`/`ViewGraph`/`GraphHost` deinit → `GraphHost.invalidate` →
`Subgraph.forEach` (an `(AnyAttribute)->Void` arg-closure → `AGSubgraphApply`) traps.
Open: the DisplayList didn't print — `renderOnce()` set up the renderer (logged
"OpenSwiftUI ViewRendererVendor: …osui") but `DisplayList.ViewRenderer.render(from:list)`
didn't emit (bare Color may produce an empty list / need real scene sizing, or the
onUpdate no-op suppressed a display-list-update pass). NEXT: (a) `*C` variant for
`Subgraph.forEach`/`AGSubgraphApply` (validate in harness first); (b) investigate the
empty emit — try a `Text`/sized view, or check whether `requestedOutputs=[.displayList]`
actually produced items. The remaining Compute walls (`AGGraphReadCachedAttribute`,
`AGGraphSearch`, `AGTupleWithBuffer`, `AGTypeApplyEnumData`/`MutableEnumData`) only get
hit by richer views.

### ⛔ EARLIER WALL (VStack probe): `signature_mismatch:swift_conformsToProtocol`
A DIFFERENT class — a **Swift runtime** function, not Compute. `OpenSwiftUICore/Runtime/
TypeConformance.swift:52` declares `@_silgen_name("swift_conformsToProtocol") func(...) ->
UnsafeRawPointer?`; the call mislowers on wasm (hit during `VStack._makeView →
TupleView._makeViewList → ProtocolDescriptor.conformance`). Candidate fix: a C shim in the
`OpenSwiftUI_SPI` C target that calls `swift_conformsToProtocol` with the correct C ABI and
is imported by Swift (mirrors the Compute *C pattern but for a runtime symbol). Needs
experimentation (its wasm calling convention is unconfirmed; SDK headers are stripped).

### Remaining likely swiftcc walls after that (args/return closures, not yet hit)
`AGGraphReadCachedAttribute`, `AGGraphSearch`, `AGTupleWithBuffer`, `AGTypeApplyEnumData`,
`AGTypeApplyMutableEnumData` (all in `/tmp/Compute`, none guarded yet).

### Phase-3 patches (regenerate /tmp from these — /tmp is ephemeral!)
- `openswiftui-phase1-wip.patch` (base `bb31b59`) — now phases 1–3 OpenSwiftUI-repo changes.
- `oag-fork.patch` (base `f20328e`) — OAG fork working tree.
- `compute-wasm.patch` (base `efb754b`) — NEW: the Compute swiftcc `*C` variants
  (Graph.swift/Metadata.swift/AnyAttribute.swift + ComputeCxx AGType/AGGraph/Graph .cpp/.h).
- `probe/` — the de-risk app (committed as files in the repo).

---

## ✅ PHASES 1+2 DONE: OpenSwiftUICore AND OpenSwiftUI compile for wasm32-wasip1
- Phase 1: `swift build --target OpenSwiftUICore … wasm` → **Build complete!** (0 errors).
- Phase 2: `swift build --target OpenSwiftUI … wasm` → **Build complete! (21.65s)** (0 errors).
The whole Foundation threading/run-loop substrate is shimmed; zero View/render errors
across both layers. Next = **phase 3** (WandrRendererHost + wire DisplayList → Option-B drawer).

### Phase 2 walls cleared (all in the patches)
- `Thread.sleep(forTimeInterval:)` added to the WASI `Thread` shim — typed `Double`
  not `TimeInterval` (Foundation is imported `internal`, so a `package` method can't
  expose the `TimeInterval` alias). Used only by test-harness loop pumping.
- `_ViewTest.loop()` / `turnRunloop()` / `turnRunLoopIfNeeded()` (Test/ViewTest.swift,
  test scaffolding shipped IN the lib) — `RunLoop.current.run(mode:before:)` is absent
  on WASI, so `#if os(WASI)`-guarded to drain `_wasmDrainMainRunLoop()` + render instead
  of pumping a (nonexistent) run loop.
- `Graph.archiveJSON(name: String?)` static added to the **OAG fork** Compute adapter
  (`oag-fork/Sources/OpenAttributeGraphShims/Adapter/Compute.swift`). AppGraph calls the
  AttributeGraph-standard static; Compute's instance `archiveJSON` is a `fatalError` stub,
  so the static is a no-op (debug launch-profiling only). Captured in `oag-fork.patch`.

## 🎯 TARGET GOAL: run **https://github.com/eleev/swiftui-2048** on wandr (Pixel 2 XL)
A real, polished, pure-SwiftUI game (the locked "real app" target). Verified suitable:
**40× `import SwiftUI`, no UIKit, no storyboards, no 3rd-party deps**; only `AudioToolbox`
(stub on wasm) + `Combine`. It's the end-to-end proof — a real SwiftUI app rendering on
the device through the whole stack.

Path to it: compile **OpenSwiftUICore** (then OpenSwiftUI) for `wasm32-wasip1` → wire the
validated DisplayList→CGContext renderer (Option B) → drop in swiftui-2048.
Full plan + scope: `docs/swift-openswiftui-wandr-feasibility.md` (phases 0–5).

## What's done (proven, pushed — see the FORKS table below for branches/SHAs)
- App+core: `harryzz/OpenSwiftUI@wasm32-wasip1` (`80d8fcf`) — phases 1–4, renders to a sink.
- Engine: `harryzz/Compute@wasm32-wasip1-osp` (`f69881b`) (AttributeGraph on wasm, reactive 42).
- `harryzz/OpenAttributeGraph@wasm32-wasip1` (`acf25d2`) (WASI un-stubs; engine = Compute backend).
- `harryzz/OpenCoreGraphics@wasm32-wasip1` (CGContext over wasi:canvas, device-verified).
- Renderer backend prototyped + device-verified: `repros/swift-canvas-spike` (P4).
- Phase 0: OpenCombine/OpenObservation build unmodified; OpenRenderBox builds (compile-only).

## 🌿 FORKS — canonical source (pushed 2026-06-19; branches match the patches)
The full wasm work is now committed to fork branches on GitHub, not just the patches.
Each branch sits on the base its patch is pinned to (older than the fork's upstream `main`):

| Repo | Branch | HEAD | Base | Carries |
|---|---|---|---|---|
| **github.com/harryzz/OpenSwiftUI** | `wasm32-wasip1` | `80d8fcf` | upstream `bb31b59` | phases 1–4 (threading shims, swiftcc fixes, `.wandr` CGContext renderer). NEW fork. `Package.resolved` reset to base (no local pins). |
| **github.com/harryzz/Compute** | `wasm32-wasip1-osp` | `f69881b` | `efb754b` | base wasm port + the phase 3/4 `AGGraphReadCachedAttributeC` `*C` variant + `cache_fetch` templatize. (older `wasm32-wasip1` branch = the jcmosc-AG-names variant) |
| **github.com/harryzz/OpenAttributeGraph** | `wasm32-wasip1` | `acf25d2` | `f20328e` | WASI un-stubs + Compute backend + adapter `archiveJSON`. |

Fresh clone of the build tree from the forks (instead of clone+patch):
```
git clone -b wasm32-wasip1     https://github.com/harryzz/OpenSwiftUI          /tmp/OpenSwiftUI
git clone -b wasm32-wasip1-osp https://github.com/harryzz/Compute             /tmp/Compute
git clone -b wasm32-wasip1     https://github.com/harryzz/OpenAttributeGraph  /tmp/oag-fork
```
The 3 base-pinned patches in this dir remain valid reproducer snapshots and match the
branches above; regenerate a patch with `git -C /tmp/<repo> diff <base>` if a branch advances.

## Build environment (the /tmp layout the OpenSwiftUI build expects)
OpenSwiftUI uses `USE_LOCAL_DEPS` → siblings at `../<Name>`. Recreate if /tmp was wiped:
```
/tmp/OpenSwiftUI            # clone of OpenSwiftUIProject/OpenSwiftUI + this patch
/tmp/OpenAttributeGraph  -> symlink to /tmp/oag-fork   (harryzz/OpenAttributeGraph, un-stubbed)
/tmp/OpenRenderBox       -> symlink to /tmp/OpenRenderBox-dep
/tmp/OpenObservation     -> symlink to /tmp/OpenObservation-dep
/tmp/OpenCoreGraphics       # upstream clone (stub CGContext compiles; wasi:canvas backend wired in phase 3)
/tmp/Compute                # harryzz/Compute on branch wasm32-wasip1-osp (OAG's Compute backend, ../Compute)
/tmp/oag-fork/Checkouts/swift -> symlink to /tmp/Compute/Submodules/swift-runtime-headers
/tmp/oag-shims/             # dispatch/syslog/openssl-sha/uint shims on -Xcc -I (from repros/compute-wasm/shims + a dispatch/dispatch.h)
```
Apply the WIP patches (both base-pinned):
- OpenSwiftUI repo (base `bb31b59`): `cd /tmp/OpenSwiftUI && git apply repros/openswiftui-wasm/openswiftui-phase1-wip.patch`
  — self-contained: CREATES `Util/WasmDispatchShim.swift` + `Util/WasmThreadingShim.swift`
  and carries the phase-1+2 OpenSwiftUICore/OpenSwiftUI edits (no manual file copy).
- OAG fork (base `f20328e`): `cd /tmp/oag-fork && git apply repros/openswiftui-wasm/oag-fork.patch`
  — the full phase-0/1/2 OAG working-tree state (un-stubs + Compute-adapter `archiveJSON`).
- (Patches and the fork branches above are now in sync — pushed 2026-06-19. Prefer cloning the
  fork branches; the patches remain as base-pinned snapshots / for regenerating against upstream.)

## The build command
```bash
cd /tmp/OpenSwiftUI
BASE=~/.swiftpm/swift-sdks/swift-6.3.2-RELEASE_wasm.artifactbundle/swift-6.3.2-RELEASE_wasm/wasm32-unknown-wasip1
OPENSWIFTUI_USE_LOCAL_DEPS=1 OPENATTRIBUTEGRAPH_OPENATTRIBUTESHIMS_COMPUTE=1 \
OPENATTRIBUTEGRAPH_USE_LOCAL_DEPS=1 \
OPENRENDERBOX_LIB_SWIFT_PATH=/tmp/oag-fork/Sources/SwiftCorelibs/include \
swift build --target OpenSwiftUICore --swift-sdk swift-6.3.2-RELEASE_wasm \
  -Xcc -I/tmp/oag-shims -Xcc -fno-exceptions -Xcc -DSWIFT_INLINE_NAMESPACE=__runtime \
  -Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN -Xcc -D_WASI_EMULATED_PROCESS_CLOCKS
```

## Walls cleared (in the patch)
- Dispatch shim (`WasmDispatchShim.swift` + guarded `import Dispatch` in AnimationListener);
  OpenCombineFoundation dep gated off non-Darwin (Package.swift); dladdr guarded
  (OpenSwiftUI_CSymbols.c); WASILibc branches (StandardLibraryAdditions.swift).
- **Threading substrate (`WasmThreadingShim.swift`)** — single-threaded WASI shims:
  - `Thread.isMainThread` → always `true` (the 2 explicit `import class Foundation.Thread`
    in StateObject/AttributeInvalidatingSubscriber are `#if !os(WASI)`-guarded; the shim
    provides a module-level `enum Thread`).
  - pthread TLS for `ThreadSpecific` — pure-Swift `pthread_key_create/getspecific/setspecific`
    over a process-global table (single thread ⇒ TLS == global). `pthread_key_t` IS visible
    from WASILibc so it's reused. Shims are `internal` (the imported C type is internal — a
    `package` func can't re-export it).
  - **RunLoop is fully `@available(*, unavailable)` on WASI** (even `RunLoop.main` traps at
    type-check) — so do NOT extend RunLoop. The two call sites (`RunLoopUtils.onNextMainRunLoop`,
    `TimerUtils.withDelay`) are `#if os(WASI)`-guarded to route through `_wasmEnqueueMainRunLoop`
    + `_wasmDrainMainRunLoop()` (a global queue the host frame loop drains in phase 3). Timers
    are no-ops until wired to the host frame clock. (The only other RunLoop user, CAHostingLayer,
    is `#if canImport(QuartzCore)` so excluded.)

## ⚠️ Phase-3 follow-ups owed by the threading shim (don't lose these)
- `_wasmDrainMainRunLoop()` must be called once per host frame, else `onNextMainRunLoop`
  deferred work (invalidations) never runs → no UI updates. It's `package`; bump to `public`
  if the host glue lives in a different Swift module.
- `withDelay` timers don't fire yet (no run loop). Wire them to the host frame clock when
  animations/timeouts are needed (phase 4).

## After OpenSwiftUICore compiles
Phase 2: OpenSwiftUI (app layer). Phase 3: a `WandrRendererHost` (model on
`StdoutRendererHost`) + wire real `DisplayList.Item` into the Option-B drawer
(`repros/swift-canvas-spike/Sources/SwiftSpike/DisplayListRenderer.swift`). Phase 4:
hand-written `Text`+`@State`+`Button` on device. Phase 5: `eleev/swiftui-2048`.
