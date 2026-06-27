import OpenAttributeGraphShims

// OAG update-machinery suite — UpdateStack / dirty-propagation / mark_changed / changedValue flags /
// mutateBody / invalidateValue. The move-4 OOB lived in the update DISPATCH; deep recursion + flags +
// in-place body mutation are the public-API levers on that machinery.

nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}
struct PlusOne: Rule { @Attribute var x: Int; var value: Int { x + 1 } }

func run() {
    let graph = Graph()
    let sg = Subgraph(graph: graph)
    Subgraph.current = sg

    // deep update recursion (1000) — bounds the UpdateStack recursion + propagation.
    var node = Attribute(value: 0); let root = node
    for _ in 0..<1000 { node = Attribute(PlusOne(x: node)) }
    check(node.value == 1000, "1000-deep update recursion")
    root.value = 5
    check(node.value == 1005, "deep recursion propagates")

    // invalidateValue → forces re-evaluation.
    let inp = Attribute(value: 3)
    let d = Attribute(PlusOne(x: inp))
    check(d.value == 4, "init = 4")
    d.invalidateValue()
    check(d.value == 4, "invalidateValue → re-eval same = 4")

    // changedValue flags: first read changed, re-read unchanged, change-after-input changed.
    let e = Attribute(value: 7)
    let f = Attribute(PlusOne(x: e))
    let r1 = f.changedValue();  check(r1.value == 8 && r1.changed,  "changedValue first read = changed")
    let r2 = f.changedValue();  check(r2.value == 8 && !r2.changed, "changedValue re-read = unchanged")
    e.value = 100
    let r3 = f.changedValue();  check(r3.value == 101 && r3.changed, "changedValue after input change = changed")

    // mutateBody: mutate a rule's stored field in place + invalidate → re-eval picks it up.
    struct Scaled: Rule { @Attribute var x: Int; var k: Int; var value: Int { x * k } }
    let xin = Attribute(value: 4)
    let scaled = Attribute(Scaled(x: xin, k: 2))
    check(scaled.value == 8, "Scaled k=2 → 8")
    scaled.mutateBody(as: Scaled.self, invalidating: true) { $0.k = 5 }
    check(scaled.value == 20, "mutateBody k=2→5 → 20")

    // wide fan-in (mark_changed must propagate to one node from many inputs).
    struct SumN: Rule { @Attribute var a: Int; @Attribute var b: Int; @Attribute var c: Int; var value: Int { a + b + c } }
    let i1 = Attribute(value: 1), i2 = Attribute(value: 2), i3 = Attribute(value: 3)
    let s = Attribute(SumN(a: i1, b: i2, c: i3))
    check(s.value == 6, "fan-in 1+2+3 = 6")
    i2.value = 20
    check(s.value == 24, "fan-in propagate one input → 24")
    i1.value = 10; i3.value = 30
    check(s.value == 60, "fan-in propagate multiple inputs → 60")

    Subgraph.current = nil
    _ = graph
}
run()
print(failures == 0 ? "ALL OAG UPDATE TESTS PASSED" : "OAG UPDATE FAILURES: \(failures)")
