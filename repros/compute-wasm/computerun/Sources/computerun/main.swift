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

// MACRO-FLIP TEST: forEachField passes a Swift closure WITH ARGS to a swiftcc C
// function. Before the AGBase.h macro flip this traps signature_mismatch:AGTypeApplyFields.
// If it prints fields, raw Swift closures work plain-C — no per-function variant needed.
struct Sample { var a: Int; var b: Bool; var c: Double }
print("forEachField test:")
forEachField(of: Sample.self) { name, offset, type in
    print("  field '\(String(cString: name))' @\(offset) : \(type)")
}
print("forEachField OK")
