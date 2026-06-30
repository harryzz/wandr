import OpenAttributeGraphShims

// OAG render-surface suite: drives the closure-ABI functions that OpenSwiftUI's RENDER/LAYOUT path
// uses but the dataflow baselines never touch — forEachField2 (DynamicProperty reflection),
// subgraphApply (ForEach traversal), readCachedAttribute (layout cache), mutateBody (@State write).
// With this green, "baseline passes" means the FULL wasm closure-ABI surface works, not just dataflow.
nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}

struct DoubleRule: Rule, Hashable {
    @Attribute var x: Int
    var value: Int { x * 2 }
}

struct ThreeFields { var a: Int; var b: Double; var c: String }

func run() {
    let graph = Graph()
    let sg = Subgraph(graph: graph)
    Subgraph.current = sg

    // 1. forEachField2 — enumerate a struct's fields (the DynamicProperty reflection path).
    var fieldCount = 0
    _ = Metadata(ThreeFields.self).forEachField(options: []) { _, _, _ in
        fieldCount += 1
        return true
    }
    check(fieldCount == 3, "forEachField2: ThreeFields has 3 fields (saw \(fieldCount))")

    // 2. subgraphApply — traverse the subgraph's flagged attributes (the ForEach traversal path).
    // forEach early-returns on empty flags (no subgraph intersection), so flag the attributes first.
    let flag = Subgraph.Flags(rawValue: 1)
    let a1 = Attribute(value: 1); a1.identifier.setFlags(flag, mask: flag)
    let a2 = Attribute(value: 2); a2.identifier.setFlags(flag, mask: flag)
    var visited = 0
    sg.forEach(flag) { _ in visited += 1 }
    check(visited >= 2, "subgraphApply: forEach visited >= 2 flagged attributes (saw \(visited))")

    // 3. readCachedAttribute — Rule.cachedValue (the layout-value cache path).
    let input = Attribute(value: 21)
    let cached = DoubleRule(x: input).cachedValue(options: [], owner: nil)
    check(cached == 42, "readCached: cachedValue(21*2) == 42 (saw \(cached))")

    // 4. mutateBody — mutate a rule attribute's body in place (the @State write path). A value
    // attribute has no rule self to mutate; a rule attribute's self IS the rule struct.
    var mutated = false
    let r = Attribute(DoubleRule(x: input))
    r.identifier.mutateBody(as: DoubleRule.self, invalidating: false) { _ in mutated = true }
    check(mutated, "mutateBody: body-mutation callback fired on a rule attribute")

    Subgraph.current = nil
    _ = (a1, a2, r)
    _ = graph
}
run()
print(failures == 0 ? "ALL OAG RENDER TESTS PASSED" : "OAG RENDER FAILURES: \(failures)")
