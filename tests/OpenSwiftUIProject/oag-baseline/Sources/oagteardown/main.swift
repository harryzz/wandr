import OpenAttributeGraphShims

// OAG subgraph-teardown / invalidation suite — PURE Compute/OAG, no OpenSwiftUI.
//
// The desktop demo crashes in `Subgraph::invalidate_now → Node::destroy → vw_destroy` at wasm addr
// 0xfffffff8 (−8): tearing down an attribute whose VALUE holds a CLASS/box reference dispatches a
// non-trivial value-witness `destroy`, and that hit corrupt metadata. The existing baseline DOES
// invalidate subgraphs (oagchurn/oagforeach) — but every attribute there holds only Int/struct, so
// the teardown only ever runs a TRIVIAL destroy. The crash needs a value whose witness `destroy`
// releases a class ref. This suite tears down class-valued attributes by the thousand.
//
// Also exercises: invalidate while a sibling reader still resolved the value this pass; mutate→re-read
// (compare + re-destroy of the class-holder); remove-then-invalidate ordering. Passes on linux AND
// wasm if Compute's teardown is correct; fails/▒-faults HERE if the value-witness-destroy path regresses.

nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}

// CLASS value → its value-witness `destroy` runs a real swift_release on teardown (the crashing path).
final class Box { let payload: [Int]; init(_ p: [Int]) { payload = p } }
struct MakeBox: Rule { @Attribute var n: Int; var value: Box { Box([n, n &* 2, n &* 3]) } }
struct ReadBox: Rule { @Attribute var b: Box; var value: Int { b.payload.reduce(0, &+) } }

// A struct that HOLDS a class ref (mirrors DynamicViewList/MutableBox-holding values whose vw_destroy
// at 0xfffffff8 was the desktop crash) — destroying it must release the inner box correctly.
struct Holder { var box: Box; var label: String }
struct MakeHolder: Rule { @Attribute var n: Int; var value: Holder { Holder(box: Box([n]), label: "h\(n)") } }
struct ReadHolder: Rule { @Attribute var h: Holder; var value: Int { (h.box.payload.first ?? -1) } }

// THE DESKTOP CRASH SHAPE: an attribute VALUE that holds a `Subgraph` reference (exactly like
// DynamicViewList / ViewList.Subgraph). The desktop crash was vw_destroy of this kind of value.
struct SGHolder { var sg: Subgraph; var tag: Int }
struct MakeSGHolder: Rule { @Attribute var n: Int; var held: Subgraph; var value: SGHolder { SGHolder(sg: held, tag: n) } }

func run() {
    let graph = Graph()
    let parent = Subgraph(graph: graph)
    var problems = 0
    let rounds = 3000

    for round in 1...rounds {
        Subgraph.current = parent
        let child = Subgraph(graph: graph)
        parent.addChild(child)
        Subgraph.current = child

        let input = Attribute(value: round)
        let box = Attribute(MakeBox(n: input))
        let sum = Attribute(ReadBox(b: box))
        let holder = Attribute(MakeHolder(n: input))
        let hread = Attribute(ReadHolder(h: holder))

        let expected = round &+ round &* 2 &+ round &* 3
        if sum.value != expected { problems += 1; if problems <= 3 { print("  MISMATCH sum r\(round): \(sum.value) != \(expected)") } }
        if hread.value != round { problems += 1; if problems <= 3 { print("  MISMATCH holder r\(round): \(hread.value) != \(round)") } }

        // mutate → re-evaluate: compares old vs new class-holding value, then re-destroys the old.
        input.value = round &* 10
        if sum.value != round &* 10 &+ round &* 20 &+ round &* 30 { problems += 1 }
        if hread.value != round &* 10 { problems += 1 }

        Subgraph.current = nil
        // TEARDOWN: remove + invalidate → invalidate_now → Node::destroy → vw_destroy of the class-holding
        // attribute values (the 0xfffffff8 path). Must release the boxes without corrupt metadata.
        parent.removeChild(child)
        child.invalidate()

        if round % 1000 == 0 { print("  teardown round \(round) ok") }
    }

    // Nested teardown: parent with a tree of children, invalidate the parent → cascade destroys all
    // class-valued descendants in one invalidate_now (the cascade path, where the desktop crash sat).
    do {
        Subgraph.current = parent
        let mid = Subgraph(graph: graph); parent.addChild(mid)
        var leaves: [Attribute<Box>] = []
        for k in 0..<50 {
            Subgraph.current = mid
            let leaf = Subgraph(graph: graph); mid.addChild(leaf)
            Subgraph.current = leaf
            leaves.append(Attribute(MakeBox(n: Attribute(value: k))))
        }
        Subgraph.current = nil
        check(leaves.allSatisfy { $0.value.payload.count == 3 }, "nested tree of 50 class-valued leaves built")
        parent.removeChild(mid)
        mid.invalidate()  // cascade-destroys mid + 50 leaves' class values
        check(true, "cascade teardown of 50 class-valued descendants survived (no vw_destroy fault)")
    }

    // THE DESKTOP CRASH SHAPE — tear down attributes whose VALUE holds a Subgraph ref.
    for round in 1...2000 {
        Subgraph.current = parent
        let target = Subgraph(graph: graph); parent.addChild(target)
        let holderSG = Subgraph(graph: graph); parent.addChild(holderSG)
        Subgraph.current = holderSG
        let holder = Attribute(MakeSGHolder(n: Attribute(value: round), held: target))
        if holder.value.tag != round { problems += 1 }
        Subgraph.current = nil
        parent.removeChild(target); target.invalidate()       // invalidate the HELD subgraph first
        parent.removeChild(holderSG); holderSG.invalidate()    // then the holder -> vw_destroy<SGHolder> releases target
    }
    check(problems == 0, "2000x teardown of Subgraph-ref-holding values (the DynamicViewList vw_destroy shape)")

    Subgraph.current = nil
    _ = graph
    if problems != 0 { failures += problems }
    print((failures == 0) ? "ALL OAG TEARDOWN TESTS PASSED (\(rounds) class-valued child subgraphs + cascade)" : "OAG TEARDOWN FAILURES: \(failures)")
}
run()
