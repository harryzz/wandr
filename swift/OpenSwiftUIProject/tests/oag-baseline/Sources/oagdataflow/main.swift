import OpenAttributeGraphShims

// OAG dataflow-correctness suite (mirrors vw-baseline/vwdataflow, but through OpenAttributeGraph's
// API over the Compute backend): rules, @Attribute dependencies, update propagation, chains,
// fan-out, multiple value types, non-trivial compare_values. Validates the OAG↔Compute integration.
nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}

struct DoubleRule: Rule { @Attribute var x: Int; var value: Int { x * 2 } }
struct PlusOneRule: Rule { @Attribute var x: Int; var value: Int { x + 1 } }
struct SumRule: Rule { @Attribute var a: Int; @Attribute var b: Int; var value: Int { a + b } }
struct ConcatRule: Rule { @Attribute var s: String; var value: String { "[\(s)]" } }

func run() {
    let graph = Graph()
    let subgraph = Subgraph(graph: graph)
    Subgraph.current = subgraph

    // 1. basic rule
    check(Attribute(DoubleRule(x: Attribute(value: 21))).value == 42, "basic rule 21*2=42")

    // 2. dependency + update propagation
    let input = Attribute(value: 10)
    let doubled = Attribute(DoubleRule(x: input))
    check(doubled.value == 20, "dep: doubled(10)=20")
    input.value = 5
    check(doubled.value == 10, "propagate: input 10->5 => doubled 10")
    input.value = 100
    check(doubled.value == 200, "propagate: input ->100 => doubled 200")

    // 3. multi-input rule
    let p = Attribute(value: 3), q = Attribute(value: 4)
    let sum = Attribute(SumRule(a: p, b: q))
    check(sum.value == 7, "two-input rule 3+4=7")
    p.value = 10
    check(sum.value == 14, "propagate one input => 10+4=14")
    q.value = 20
    check(sum.value == 30, "propagate other input => 10+20=30")

    // 4. multi-level chain a -> b(a*2) -> c(b+1)
    let a = Attribute(value: 3)
    let b = Attribute(DoubleRule(x: a))
    let c = Attribute(PlusOneRule(x: b))
    check(c.value == 7, "chain a=3 -> b=6 -> c=7")
    a.value = 10
    check(c.value == 21, "chain propagate a=10 -> c=21")

    // 5. fan-out: one input, two dependents
    let shared = Attribute(value: 7)
    let d1 = Attribute(DoubleRule(x: shared)), d2 = Attribute(PlusOneRule(x: shared))
    check(d1.value == 14 && d2.value == 8, "fan-out 7 -> 14,8")
    shared.value = 1
    check(d1.value == 2 && d2.value == 2, "fan-out propagate 1 -> 2,2")

    // 6. string value type
    let sIn = Attribute(value: "hi"); let sOut = Attribute(ConcatRule(s: sIn))
    check(sOut.value == "[hi]", "string rule")
    sIn.value = "world"
    check(sOut.value == "[world]", "string propagate")

    // 7. deep chain (100) + propagation
    var node = Attribute(value: 0); let root = node
    for _ in 0..<100 { node = Attribute(PlusOneRule(x: node)) }
    check(node.value == 100, "100-deep chain = 100")
    root.value = 1000
    check(node.value == 1100, "100-deep chain propagate = 1100")

    // 8. NON-TRIVIAL intermediate value -> exercises compare_values change-detection through a chain.
    struct Pair { var a: String; var b: [Int] }
    struct MakePair: Rule { @Attribute var n: Int; var value: Pair { Pair(a: "v\(n)", b: [n, n &* 2]) } }
    struct ReadPair: Rule { @Attribute var p: Pair; var value: String { "\(p.a):\(p.b.count):\(p.b.last ?? -1)" } }
    let nIn = Attribute(value: 1)
    let pairAttr = Attribute(MakePair(n: nIn))
    let reader = Attribute(ReadPair(p: pairAttr))
    check(reader.value == "v1:2:2", "non-trivial chain initial (v1:2:2)")
    nIn.value = 2
    check(reader.value == "v2:2:4", "non-trivial PROPAGATION through compare_values (expect v2:2:4)")
    nIn.value = 9
    check(reader.value == "v9:2:18", "non-trivial PROPAGATION again (expect v9:2:18)")

    Subgraph.current = nil
    _ = graph
}
run()
print(failures == 0 ? "ALL OAG DATAFLOW TESTS PASSED" : "OAG DATAFLOW FAILURES: \(failures)")
