import OpenAttributeGraphShims

// OAG value-type lifecycle suite — PURE Compute/OAG. Drives EVERY value-witness category through
// value_set (compare_values + the destroy on re-eval): trivial, struct, CLASS-holding, EXISTENTIAL,
// ENUM (1- and 2-payload), OPTIONAL, TUPLE, LARGE (>0x10 → alloc_bytes), STRING, nested. The baseline
// only ever used Int/String/one struct, so the non-trivial value-witness paths (where the wasm crashes
// lived) were untested. Uses the public `Map` closure-rule so one driver covers all types.

nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}

// make(a)->T, read(T)->Int; set input a->b->b: change must PROPAGATE (compare saw it) + same-value stable.
func cycle<T>(_ name: String, _ a: Int, _ b: Int, _ make: @escaping (Int) -> T, _ read: @escaping (T) -> Int) {
    let input = Attribute(value: a)
    let made = Attribute(Map(input, make))      // Attribute<T>
    let reader = Attribute(Map(made, read))      // Attribute<Int>
    check(reader.value == read(make(a)), "\(name): initial = \(read(make(a)))")
    input.value = b
    check(reader.value == read(make(b)), "\(name): change \(a)->\(b) PROPAGATES")
    input.value = b
    check(reader.value == read(make(b)), "\(name): same-value set stays correct")
}

final class Box { let xs: [Int]; init(_ xs: [Int]) { self.xs = xs } }
protocol P {}
struct PLeaf: P { var n: Int }
enum E { case none; case one(Int); case two(Int, String) }
struct Big { var a = 0, b = 0, c = 0, d = 0, e = 0, f = 0; var s = "" }   // > 0x10 → alloc_bytes path

func run() {
    let graph = Graph()
    let sg = Subgraph(graph: graph)
    Subgraph.current = sg

    cycle("Int",         1, 2, { $0 },                         { $0 })
    cycle("Bool",        1, 0, { $0 != 0 },                    { $0 ? 1 : 0 })
    cycle("Double",      3, 7, { Double($0) * 1.5 },           { Int($0) })
    cycle("String",      4, 8, { "v\($0)" },                   { Int($0.dropFirst()) ?? -1 })
    cycle("Tuple",       5, 9, { ($0, "t\($0)") },             { $0.0 })
    cycle("Struct(Big)", 6, 1, { Big(a: $0, b: $0 * 2, s: "s\($0)") }, { $0.a + $0.b })
    cycle("Optional",    7, 0, { $0 == 0 ? nil : $0 },         { $0 ?? -1 })
    cycle("Class",       8, 2, { Box([$0, $0 * 2]) },          { $0.xs.reduce(0, +) })
    cycle("Existential", 9, 3, { PLeaf(n: $0) as P },          { ($0 as? PLeaf)?.n ?? -1 })
    cycle("Enum1",       1, 4, { E.one($0) },                  { if case let .one(x) = $0 { return x }; return -1 })
    cycle("Enum2",       1, 4, { E.two($0, "e\($0)") },        { if case let .two(x, _) = $0 { return x }; return -1 })
    cycle("Array",       2, 6, { Array(0..<$0) },              { $0.count })
    cycle("NestedClass", 3, 5, { (Box([$0]), $0) },            { $0.0.xs.first ?? -1 })

    // keyPath / dynamicMember projection (Attribute<Value> -> Attribute<Member>)
    struct Pt { var x: Int; var y: Int }
    let nIn = Attribute(value: 3)
    let pt = Attribute(Map(nIn) { Pt(x: $0, y: $0 * 10) })
    let yProj = pt.y
    check(yProj.value == 30, "dynamicMember projection pt.y = 30")
    nIn.value = 5
    check(yProj.value == 50, "dynamicMember projection propagates -> 50")

    // setValue / changedValue / invalidateValue
    let s = Attribute(value: 1)
    _ = s.setValue(9)
    check(s.value == 9, "setValue -> 9")
    check(s.changedValue().value == 9, "changedValue.value = 9")

    Subgraph.current = nil
    _ = graph
}
run()
print(failures == 0 ? "ALL OAG VALUES TESTS PASSED" : "OAG VALUES FAILURES: \(failures)")
