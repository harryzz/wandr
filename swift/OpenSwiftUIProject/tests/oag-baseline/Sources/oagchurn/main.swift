import OpenAttributeGraphShims

// OAG churn/stress: many subgraphs, each builds an attribute chain + propagates, through OpenAttributeGraph's
// public API over the Compute backend. Exercises subgraph creation + the update/invalidation machinery
// (the &->| deferred-invalidation path) under volume — the pattern that surfaced the original Subgraph UAF.
struct PlusOneRule: Rule { @Attribute var x: Int; var value: Int { x + 1 } }

func run() {
    let graph = Graph()
    var problems = 0
    let rounds = 3000
    for round in 1...rounds {
        let sg = Subgraph(graph: graph)
        Subgraph.current = sg
        let input = Attribute(value: round)
        var node = input
        for _ in 0..<8 { node = Attribute(PlusOneRule(x: node)) }
        if node.value != round + 8 { problems += 1; if problems <= 3 { print("  MISMATCH r\(round): \(node.value) != \(round + 8)") } }
        input.value = round &* 2
        if node.value != (round &* 2) + 8 { problems += 1 }
        Subgraph.current = nil
        if round % 1000 == 0 { print("  churn round \(round) ok") }
    }
    _ = graph
    print(problems == 0 ? "ALL OAG CHURN TESTS PASSED (\(rounds) subgraphs)" : "OAG CHURN PROBLEMS: \(problems)")
}
run()
