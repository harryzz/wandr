import OpenAttributeGraphShims

// OAG weak / optional / indirect reference suite — PURE Compute/OAG, no OpenSwiftUI.
// Covers WeakAttribute, OptionalAttribute, IndirectAttribute — the move-2 "reading from invalid source
// attribute" family (a weak/indirect ref to an attribute whose subgraph was invalidated, and the
// subgraph_id-vs-zone_id false-expiry). The baseline never created a weak/indirect ref at all.

nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}

func run() {
    let graph = Graph()
    let sg = Subgraph(graph: graph)
    Subgraph.current = sg

    // --- WeakAttribute ---
    let a = Attribute(value: 10)
    let w = WeakAttribute(a)
    check(w.value == 10, "WeakAttribute -> live value 10")
    a.value = 20
    check(w.value == 20, "WeakAttribute sees update -> 20")
    let wEmpty = WeakAttribute<Int>()
    check(wEmpty.value == nil, "empty WeakAttribute -> nil")

    // --- OptionalAttribute ---
    let opt = OptionalAttribute(a)
    check(opt.value == 20, "OptionalAttribute -> 20")
    let optNil = OptionalAttribute<Int>()
    check(optNil.value == nil, "empty OptionalAttribute -> nil")
    let optW = OptionalAttribute(w)
    check(optW.value == 20, "OptionalAttribute(weak) -> 20")

    // --- IndirectAttribute (redirect + resetSource) ---
    let src1 = Attribute(value: 100)
    let ind = IndirectAttribute(source: src1)
    check(ind.attribute.value == 100, "IndirectAttribute -> source 100")
    let src2 = Attribute(value: 200)
    ind.source = src2
    check(ind.attribute.value == 200, "IndirectAttribute re-source -> 200")

    // --- THE MOVE-2 FAMILY: weak ref across subgraph invalidation, + false-expiry guard ---
    Subgraph.current = sg
    let child = Subgraph(graph: graph); sg.addChild(child)
    Subgraph.current = child
    let inChild = Attribute(value: 42)
    let wChild = WeakAttribute(inChild)
    Subgraph.current = sg
    check(wChild.value == 42, "weak -> attr in LIVE child = 42")

    // A LIVE weak ref must NOT falsely expire while OTHER subgraphs are created+invalidated (this is the
    // subgraph_id/zone_id divergence that made move-2's weak source read as expired). Churn 1000 zones.
    var falseExpiry = 0
    for _ in 0..<1000 {
        let t = Subgraph(graph: graph); sg.addChild(t)
        sg.removeChild(t); t.invalidate()       // bump the zone counter
        if wChild.value != 42 { falseExpiry += 1 }
    }
    check(falseExpiry == 0, "LIVE weak ref does NOT falsely expire under 1000 zone churns")

    // Now invalidate the child the weak ref points into → it must expire to nil (NOT fault).
    sg.removeChild(child); child.invalidate()
    check(wChild.value == nil, "weak -> attr in INVALIDATED child = nil (correctly expired, no fault)")

    Subgraph.current = nil
    _ = graph
}
run()
print(failures == 0 ? "ALL OAG WEAKREF TESTS PASSED" : "OAG WEAKREF FAILURES: \(failures)")
