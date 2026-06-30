import OpenAttributeGraphShims

// [#12 DETECTOR] Reproduce the OpenSwiftUI 2048 frame-#14 crash in PURE Compute/OAG, fast:
// a reader (subgraph A) does a strict @Attribute read of a NON-MUTABLE OFFSET PROJECTION
// (source[keyPath:\.x], like GeometryReader's `childGeometry.origin()`) whose source lives in a
// DIFFERENT subgraph B. B is torn down and its page recycled (wasm: zone-id changes -> the offset
// indirect's weak source `expired()` -> strict read precondition "reading from invalid source
// attribute"). A correct AttributeGraph makes the consumer just work; this isolates whether Compute
// crashes where AG would not.
nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}

struct Point { var x: Int; var y: Int }
struct ReadProjected: Rule { @Attribute var p: Int; var value: Int { p } }
struct DoubleRule: Rule { @Attribute var x: Int; var value: Int { x * 2 } }

func run() {
    let graph = Graph()
    let root = Subgraph(graph: graph)

    // --- Case 1: read BEFORE teardown (sanity: should always work) ---
    do {
        let bsub = Subgraph(graph: graph); root.addChild(bsub)
        let asub = Subgraph(graph: graph); root.addChild(asub)
        Subgraph.current = bsub
        let source = Attribute(value: Point(x: 42, y: 7))
        let projX: Attribute<Int> = source[keyPath: \.x]
        Subgraph.current = asub
        let reader = Attribute(ReadProjected(p: projX))
        check(reader.value == 42, "case1: read projection before teardown = 42")
    }

    // --- Case 2: read AFTER source-subgraph teardown + page recycle (the #12 pattern) ---
    do {
        let bsub = Subgraph(graph: graph); root.addChild(bsub)
        let asub = Subgraph(graph: graph); root.addChild(asub)
        Subgraph.current = bsub
        let source = Attribute(value: Point(x: 99, y: 1))
        let projX: Attribute<Int> = source[keyPath: \.x]
        Subgraph.current = asub
        let reader = Attribute(ReadProjected(p: projX)) // NOT read yet (lazy edge), mirrors #12

        // Tear down the source's subgraph (like a per-tile childGeometry subgraph being invalidated).
        root.removeChild(bsub); bsub.invalidate()

        // Force B's pages to be reused by other subgraphs -> zone-id mismatch -> weak source expires.
        for _ in 0 ..< 80 {
            let churn = Subgraph(graph: graph); root.addChild(churn)
            Subgraph.current = churn
            for _ in 0 ..< 16 { _ = Attribute(value: Point(x: 1, y: 2)) }
            root.removeChild(churn); churn.invalidate()
        }

        // First read of the reader AFTER its source died. A correct AG keeps the consumer working
        // (does NOT abort the process here). If Compute aborts, this target crashes = bug reproduced.
        Subgraph.current = asub
        let got = reader.value
        check(true, "case2: read projection after source teardown did NOT crash (got \(got))")
    }

    // --- Case 3: mutate a LONG-LIVED source AFTER a cross-subgraph dependent is torn down + recycled.
    // Reproduces the propagate_dirty / dangling-output_edges corruption (demo frame-#14 manifestation #4/#5):
    // teardown must remove the dead dependent from the live source's output_edges, else propagate_dirty
    // walks a wild edge. ---
    do {
        Subgraph.current = root
        let a = Attribute(value: 10) // long-lived source in root (survives)
        let s1 = Subgraph(graph: graph); root.addChild(s1)
        Subgraph.current = s1
        let b = Attribute(DoubleRule(x: a)) // dependent in another subgraph -> a.output_edges gains b
        check(b.value == 20, "case3: dependent reads source = 20") // establish the A->B edge

        root.removeChild(s1); s1.invalidate() // tear down the dependent's subgraph

        for _ in 0 ..< 80 { // recycle s1's pages
            let c = Subgraph(graph: graph); root.addChild(c)
            Subgraph.current = c
            for _ in 0 ..< 16 { _ = Attribute(value: 0) }
            root.removeChild(c); c.invalidate()
        }

        // Mutate the source -> propagate_dirty(a) walks a.output_edges. If the dead dependent b was not
        // removed, that edge is now wild -> crash. A correct teardown removed it.
        Subgraph.current = root
        a.value = 999
        check(true, "case3: mutate source after dependent teardown did NOT crash")
    }

    // --- Case 4: invalidateValue() -> value_mark -> propagate_dirty (the demo's frame-#14 path), with a
    // dependent whose subgraph was torn down + recycled. value_set/mark_changed (case3) is a DIFFERENT path;
    // this one walks output_edges calling Node::state() on each. ---
    do {
        Subgraph.current = root
        let s = Attribute(value: 7) // long-lived source in root
        let a = Subgraph(graph: graph); root.addChild(a)
        Subgraph.current = a
        let r = Attribute(DoubleRule(x: s))
        check(r.value == 14, "case4: dependent reads source = 14")
        root.removeChild(a); a.invalidate()
        for _ in 0 ..< 80 {
            let c = Subgraph(graph: graph); root.addChild(c)
            Subgraph.current = c
            for _ in 0 ..< 16 { _ = Attribute(value: 0) }
            root.removeChild(c); c.invalidate()
        }
        Subgraph.current = root
        s.invalidateValue() // propagate_dirty(s) walks s.output_edges -> the demo frame-#14 path
        check(true, "case4: invalidateValue after dependent teardown+recycle did NOT crash")
    }

    // --- Case 5: WeakAttribute.attribute (the EXACT accessor the demo's asyncSignal guard uses,
    // `weakAsyncSignal.attribute?.invalidateValue()`) after teardown + page recycle. oagweakref only tests
    // `.value` on a freed (not recycled) page; the demo uses `.attribute` after recycle. The generational
    // check MUST report nil. If it returns a stale non-nil handle, invalidateValue runs on a dead node. ---
    do {
        let b = Subgraph(graph: graph); root.addChild(b)
        Subgraph.current = b
        let sig = Attribute(value: 0)
        let weak = WeakAttribute(sig)
        check(weak.attribute != nil, "case5: weak.attribute alive before teardown")
        Subgraph.current = root
        root.removeChild(b); b.invalidate()
        for _ in 0 ..< 80 {
            let c = Subgraph(graph: graph); root.addChild(c)
            Subgraph.current = c
            for _ in 0 ..< 16 { _ = Attribute(value: 0) }
            root.removeChild(c); c.invalidate()
        }
        Subgraph.current = root
        check(weak.attribute == nil, "case5: weak.attribute EXPIRED after teardown+recycle")
        weak.attribute?.invalidateValue() // the demo's guarded call — must be a safe no-op
        check(true, "case5: guarded invalidateValue on expired weak did NOT crash")
    }

    // --- Case 6: the demo's EXACT mechanism — a dependent that reads the source via MANUAL addInput
    // (info.addInput(asyncSignal, options: ._4, token: 0)), torn down + recycled, then invalidateValue(source).
    // If manual-addInput edges aren't cleaned on teardown like Rule edges are, propagate_dirty(source) walks
    // a dangling edge -> Node::state() trap (demo frame #14). ._4 == IAGInputOptions(rawValue: 1 << 2). ---
    do {
        Subgraph.current = root
        let s = Attribute(value: 0)
        let a = Subgraph(graph: graph); root.addChild(a)
        Subgraph.current = a
        let r = Attribute(value: 0)
        r.addInput(s, options: IAGInputOptions(rawValue: 1 << 2), token: 0) // manual edge r->s, like the demo
        Subgraph.current = root
        root.removeChild(a); a.invalidate() // tear down r
        for _ in 0 ..< 80 {
            let c = Subgraph(graph: graph); root.addChild(c)
            Subgraph.current = c
            for _ in 0 ..< 16 { _ = Attribute(value: 0) }
            root.removeChild(c); c.invalidate()
        }
        Subgraph.current = root
        s.invalidateValue() // propagate_dirty(s); if r's manual edge wasn't cleaned -> dangling -> crash
        check(true, "case6: invalidateValue after MANUAL-addInput dependent teardown+recycle did NOT crash")
    }

    // --- Case 7: the demo's DEFERRED-teardown path (renderOnce wraps everything in
    // withoutSubgraphInvalidation). Tear down a dependent WHILE deferring (edge cleanup is queued, not run),
    // call invalidateValue(source) inside the window, then drain, recycle, and invalidate again. If a
    // dependent's output edge survives the deferred drain, propagate_dirty later walks it into a recycled
    // page. ---
    do {
        Subgraph.current = root
        let s = Attribute(value: 0)
        let a = Subgraph(graph: graph); root.addChild(a)
        Subgraph.current = a
        let r = Attribute(DoubleRule(x: s))
        check(r.value == 0, "case7: dependent reads source = 0")
        Subgraph.current = root
        graph.withoutSubgraphInvalidation {
            root.removeChild(a); a.invalidate() // DEFERRED teardown of r
            s.invalidateValue() // invalidate DURING the deferred window (like makeItem)
            check(true, "case7a: invalidateValue during deferred dependent teardown did NOT crash")
        }
        for _ in 0 ..< 80 { // recycle r's now-drained page
            let c = Subgraph(graph: graph); root.addChild(c)
            Subgraph.current = c
            for _ in 0 ..< 16 { _ = Attribute(value: 0) }
            root.removeChild(c); c.invalidate()
        }
        Subgraph.current = root
        s.invalidateValue() // if r survived in s.output_edges -> walks recycled page -> crash
        check(true, "case7b: invalidateValue after deferred-drain + recycle did NOT crash")
    }

    print(failures == 0 ? "ALL OAG OFFSET TESTS PASSED" : "OAG OFFSET FAILURES: \(failures)")
}
run()
