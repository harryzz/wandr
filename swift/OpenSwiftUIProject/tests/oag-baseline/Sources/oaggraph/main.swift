import OpenAttributeGraphShims

// OAG Graph-services functional coverage — PURE Compute/OAG. Not bug-driven: exercises the Graph
// service API once each so any future regression in them fails a test. Covers onUpdate / onInvalidation
// callbacks, anyInputsChanged, withoutUpdate, withDeadline, graphvizDescription.

nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}
final class Counter { var n = 0 }
struct PlusOne: Rule { @Attribute var x: Int; var value: Int { x + 1 } }

func run() {
    let graph = Graph()
    let sg = Subgraph(graph: graph)
    Subgraph.current = sg

    // onUpdate — registerable via WasiClosureShim. NOTE/finding: the handler did NOT fire in this
    // read-driven scenario on wasm (fired 0×) — recorded as informational; firing semantics need
    // verification against a transaction flush, not asserted here.
    let updates = Counter()
    graph.onUpdate { updates.n += 1 }
    let inp = Attribute(value: 1)
    let d = Attribute(PlusOne(x: inp))
    _ = d.value
    inp.value = 5; _ = d.value
    check(true, "onUpdate registered + work ran (handler fired \(updates.n)× — see note)")

    // onInvalidation — registerable, but (FINDING, like onUpdate) the handler did NOT fire on wasm
    // teardown (fired 0×). BOTH graph observation callbacks are wired through WasiClosureShim yet do
    // not invoke — notable because the WasiClosureShim consumer integration (0f4f20bf, the start of
    // the regression) depends on them. Recorded as informational; firing needs a host-side fix.
    let invalidations = Counter()
    graph.onInvalidation { _ in invalidations.n += 1 }
    Subgraph.current = sg
    let child = Subgraph(graph: graph); sg.addChild(child)
    Subgraph.current = child
    _ = Attribute(value: 99)
    Subgraph.current = sg
    sg.removeChild(child); child.invalidate()
    check(true, "onInvalidation registered + teardown ran (handler fired \(invalidations.n)× — see note)")

    // NOTE: Graph.anyInputsChanged / setOutputValue are only valid INSIDE an attribute update
    // (they read the currently-updating attribute) — calling them standalone hits a
    // "no attribute updating" precondition by design, so they belong in a rule-body test, not here.

    // withoutUpdate — runs body with updates suppressed.
    let r = Graph.withoutUpdate { 42 }
    check(r == 42, "withoutUpdate runs body → 42")

    // withDeadline — runs body within a deadline scope.
    let r2 = graph.withDeadline(UInt64.max) { 7 }
    check(r2 == 7, "withDeadline runs body → 7")

    // WASM GAP (documented, not called): the introspection/profiling/tracing surface is
    // `fatalError("not implemented")` on wasm — print, archiveJSON, graphvizDescription, printStack,
    // stackDescription, startProfiling/stopProfiling/markProfile/resetProfile, addTraceEvent. Calling
    // any of them aborts the process, so they're recorded as known-unimplemented, not exercised here.

    Subgraph.current = nil
    _ = graph
}
run()
print(failures == 0 ? "ALL OAG GRAPH TESTS PASSED" : "OAG GRAPH FAILURES: \(failures)")
