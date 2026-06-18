import Compute

let graph = Graph()
let subgraph = Subgraph(graph: graph)
Subgraph.current = subgraph

// A RULE attribute: its value is computed by update(), which reads an input attribute.
struct DoubleRule: Rule {
    @Attribute var input: Int
    var value: Int { input * 2 }
}

let inputAttr = Attribute(value: 21)
let ruleAttr = Attribute(DoubleRule(input: inputAttr))
// Reading .value drives the graph to UPDATE ruleAttr -> invokes the C-CC _update
// trampoline -> runs DoubleRule.value -> reads input (21) -> 42.
print("Compute rule on wasi: ruleAttr.value = \(ruleAttr.value)")
