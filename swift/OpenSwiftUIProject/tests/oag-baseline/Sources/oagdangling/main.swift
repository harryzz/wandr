import OpenAttributeGraphShims

// oag-baseline: dangling CROSS-SUBGRAPH edge teardown UAF — PURE Compute/OAG, no OpenSwiftUI, no visuals.
//
// Reproduces the ROOT of the intermittent AttributeGraph node use-after-free that crashes the
// eleev/swiftui-2048 app under heavy dynamic-view churn (side menu / modals / Settings↔About↔game swaps).
//
// Unlike `oagteardown` (which tears down class-VALUED attributes to exercise `vw_destroy`), this
// reproduces crash B specifically: a SURVIVING parent attribute A has a dependent B in a CHILD subgraph
// (a cross-subgraph edge A → B). Invalidating the child is exactly what `DynamicContainer.eraseItem` does
// on every dynamic-view removal (`maxUnusedItems` defaults to 0, so the pool path is never taken). Then
// A is dirtied — exactly what `DynamicAnimationListener.animationWasRemoved` does via
// `asyncSignal.invalidateValue()` — and `value_mark → propagate_dirty` walks A's output_edges, which
// still references the now-FREED B (the dangling edge). Without the propagate_dirty liveness guard this
// reads freed/reused memory and traps on a wild pointer; with it the dead edge is skipped and the
// surviving dependent C still recomputes.
//
// Passes trivially on the immortal linux platform (nothing is freed); reproduces on wasm (faithful free).
// See .claude/memory/reference-openswiftui-immortal-fix.

nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}

// B and C both DEPEND on A → reading them establishes the edges A -> B and A -> C in A's output_edges.
struct Derived: Rule {
    @Attribute var input: Int
    var value: Int { input &+ 1 }
}

func run() {
    let graph = Graph()
    let parent = Subgraph(graph: graph)
    let rounds = 2000
    var mismatches = 0

    for round in 1...rounds {
        // A (settable, drives propagate_dirty) and C both live in the PARENT and SURVIVE the round.
        Subgraph.current = parent
        let a = Attribute(value: round)
        let c = Attribute(Derived(input: a))

        // B lives in a CHILD subgraph and depends on A → cross-subgraph edge A -> B.
        let child = Subgraph(graph: graph)
        parent.addChild(child)
        Subgraph.current = child
        let b = Attribute(Derived(input: a))

        // Evaluate both so A -> B and A -> C are live edges in A's output_edges.
        if b.value != round &+ 1 { mismatches += 1 }
        if c.value != round &+ 1 { mismatches += 1 }

        // Tear down the child exactly like DynamicContainer.eraseItem's invalidate path (frees B on wasm).
        Subgraph.current = parent
        parent.removeChild(child)
        child.invalidate()

        // Dirty A → mark_changed → propagate_dirty walks A's output_edges, including the now-dangling
        // A -> B. THE CRASH POINT: without the fix this reads the freed B and faults on a wild pointer.
        a.value = round &* 10

        // Reaching here means the dead edge was skipped/severed. The surviving dependent must recompute.
        if c.value != round &* 10 &+ 1 { mismatches += 1 }

        Subgraph.current = nil
        if round % 500 == 0 { print("  dangling-edge teardown round \(round) survived") }
    }

    check(mismatches == 0,
          "\(rounds)x invalidate-child-then-propagate over a dangling cross-subgraph edge (no UAF; surviving dependent correct)")
    _ = graph
    print((failures == 0) ? "ALL OAG DANGLING-EDGE TESTS PASSED" : "OAG DANGLING-EDGE FAILURES: \(failures)")
}

run()
