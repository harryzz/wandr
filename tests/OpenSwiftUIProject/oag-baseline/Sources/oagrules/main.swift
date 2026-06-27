import OpenAttributeGraphShims

// OAG Rule-kinds functional coverage — PURE Compute/OAG. Not bug-driven: exercises the rule variants
// beyond the plain `Rule` used everywhere else — `StatefulRule` (carries state across evaluations via
// context.value + hasValue) and `Map` (closure rule). Plain `Rule` is already covered by oagdataflow.

nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}

// StatefulRule: accumulate the running sum of `input` across evaluations (state survives via `value`).
struct Accumulator: StatefulRule {
    @Attribute var input: Int
    typealias Value = Int
    static var initialValue: Int? { 0 }
    mutating func updateValue() {
        let prev = hasValue ? value : 0
        value = prev + input
    }
}

// StatefulRule: latch the maximum seen.
struct MaxLatch: StatefulRule {
    @Attribute var input: Int
    typealias Value = Int
    static var initialValue: Int? { Int.min }
    mutating func updateValue() {
        let prev = hasValue ? value : Int.min
        value = max(prev, input)
    }
}

func run() {
    let graph = Graph()
    let sg = Subgraph(graph: graph)
    Subgraph.current = sg

    // StatefulRule — accumulator carries state across input changes.
    let inp = Attribute(value: 5)
    let acc = Attribute(Accumulator(input: inp))
    check(acc.value == 5, "StatefulRule accumulate: first = 5")
    inp.value = 3
    check(acc.value == 8, "StatefulRule accumulate: +3 → 8")
    inp.value = 10
    check(acc.value == 18, "StatefulRule accumulate: +10 → 18")

    // StatefulRule — max latch holds the peak.
    let m = Attribute(value: 4)
    let latch = Attribute(MaxLatch(input: m))
    check(latch.value == 4, "MaxLatch: 4")
    m.value = 9
    check(latch.value == 9, "MaxLatch: 9")
    m.value = 2
    check(latch.value == 9, "MaxLatch holds peak at 9 (input dropped to 2)")

    // Map closure-rule chained.
    let n = Attribute(value: 6)
    let mapped = Attribute(Map(n) { $0 * $0 })
    check(mapped.value == 36, "Map closure rule 6² = 36")
    n.value = 7
    check(mapped.value == 49, "Map propagates 7² = 49")

    Subgraph.current = nil
    _ = graph
}
run()
print(failures == 0 ? "ALL OAG RULES TESTS PASSED" : "OAG RULES FAILURES: \(failures)")
