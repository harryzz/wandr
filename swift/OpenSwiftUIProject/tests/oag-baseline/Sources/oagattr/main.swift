import OpenAttributeGraphShims

// OAG Attribute-operations functional coverage — PURE Compute/OAG. Not bug-driven: exercises the
// Attribute projection/observation/validation API once each: dynamicMember + keyPath subscripts,
// valueAndFlags, prefetchValue, validate, invalidateValue, setValue/changedValue, hash/identity.

nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}
struct Pt { var x: Int; var y: Int }

func run() {
    let graph = Graph()
    let sg = Subgraph(graph: graph)
    Subgraph.current = sg

    let inp = Attribute(value: 3)
    let pt = Attribute(Map(inp) { Pt(x: $0, y: $0 * 10) })

    // dynamicMember projection (Attribute<Value> → Attribute<Member>) — both members.
    check(pt.x.value == 3, "dynamicMember pt.x = 3")
    check(pt.y.value == 30, "dynamicMember pt.y = 30")
    inp.value = 4
    check(pt.x.value == 4 && pt.y.value == 40, "dynamicMember projections propagate (4, 40)")
    // keyPath subscript projection — previously logged as trapping "non-direct attribute id" on wasm;
    // re-verified 2026-06-28 to work on wasm AND linux for stored-property keyPaths (offset(of:)
    // resolves → direct unsafeOffset) and for computed/no-offset keyPaths (→ Focus indirect, reads fine).
    check(pt[keyPath: \.x].value == 4, "keyPath pt[\\.x] = 4")
    check(pt[keyPath: \.y].value == 40, "keyPath pt[\\.y] = 40")

    // valueAndFlags / prefetchValue / validate / invalidateValue on a DIRECT attribute.
    let vf = inp.valueAndFlags(options: IAGValueOptions())
    check(vf.value == 4, "valueAndFlags.value = 4")
    inp.prefetchValue()
    inp.validate()
    check(true, "prefetchValue + validate ran without fault")
    inp.invalidateValue()
    check(inp.value == 4, "invalidateValue → re-read = 4")

    // setValue / changedValue on an input
    let s = Attribute(value: 1)
    let did = s.setValue(9)
    check(s.value == 9, "setValue → 9 (changed=\(did))")
    check(s.changedValue().value == 9, "changedValue.value = 9")

    // hashing / Hashable conformance (attributes are identity-hashable)
    var hasher = Hasher(); inp.hash(into: &hasher)
    check(inp == inp && inp != s, "Attribute identity equality")

    Subgraph.current = nil
    _ = graph
}
run()
print(failures == 0 ? "ALL OAG ATTR TESTS PASSED" : "OAG ATTR FAILURES: \(failures)")
