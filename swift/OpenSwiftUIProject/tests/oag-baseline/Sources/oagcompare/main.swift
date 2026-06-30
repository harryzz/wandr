import OpenAttributeGraphShims

// OAG comparison-engine suite — PURE Compute/OAG, no OpenSwiftUI.
//
// Both of this session's worst wasm bugs lived in Compute's value comparison (LayoutDescriptor
// `compare_existential_values`), and the EXISTING baseline missed them because every other test
// compares only Int/String/struct — never an EXISTENTIAL (`any P`). This suite compares existentials:
//
//  (A) FREEZE: a copy-paste `lhs`→`rhs` made every existential compare equal-to-ITSELF, so change
//      detection never fired through `any View`/`any ViewList` attributes → the board froze. Guard:
//      two DIFFERENT existential values must compare NOT-equal so a dependent re-evaluates.
//  (B) RECURSION→CRASH: `compare_existential_values` fetched the EXISTENTIAL CONTAINER's layout and
//      walked it over the projected payload — whose first entry is again an Existential — re-entering
//      itself unboundedly → wasm shadow-stack overflow into the Swift metadata pool → null/▒ funcref
//      value-witness `destroy` (`uninitialized element`). Fix = compare the projected payload against
//      its DYNAMIC type's layout/size. Guard: comparing SELF-NESTING existentials must TERMINATE.
//
// If Compute is correct this passes on linux AND wasm; if the existential compare regresses it fails
// HERE (or shadow-stack-overflows), at the right layer — so Compute can be blessed in isolation.

nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}

protocol Shape {}
struct Leaf: Shape { var n: Int }
// SELF-NESTING existential: a Shape whose field is itself `any Shape` — the exact shape (WrappedList.base:
// any ViewList) that recursed forever in the comparison engine.
struct Wrapper: Shape { var inner: any Shape; var seed: Int }

// Rules whose VALUE is an existential (`any Shape`) — recomputed when an input changes, so each
// re-evaluation runs compare_existential_values(old, new).
struct MakeLeaf: Rule { @Attribute var n: Int; var value: any Shape { Leaf(n: n) } }
struct MakeWrapper: Rule {
    @Attribute var seed: Int
    @Attribute var inner: any Shape
    var value: any Shape { Wrapper(inner: inner, seed: seed) }
}
// Reader projects the existential to an Int so change-detection is observable downstream.
struct ReadShape: Rule {
    @Attribute var s: any Shape
    var value: Int {
        switch s {
        case let l as Leaf: return l.n
        case let w as Wrapper: return w.seed &* 1000 &+ ((w.inner as? Leaf)?.n ?? -1)
        default: return -999
        }
    }
}

func run() {
    let graph = Graph()
    let sg = Subgraph(graph: graph)
    Subgraph.current = sg

    // (A) FREEZE GUARD — different existential values must NOT compare equal → change propagates.
    let n = Attribute(value: 5)
    let leaf = Attribute(MakeLeaf(n: n))            // any Shape = Leaf(5)
    let reader = Attribute(ReadShape(s: leaf))
    check(reader.value == 5, "existential read: Leaf(5) -> 5")
    n.value = 9
    check(reader.value == 9, "FREEZE GUARD: existential change 5->9 PROPAGATES (lhs->rhs bug would stick at 5)")
    n.value = 5
    check(reader.value == 5, "existential change back 9->5 propagates")
    n.value = 5  // set to the SAME value → must compare EQUAL (no spurious change), reader stays 5
    check(reader.value == 5, "existential same-value set compares EQUAL (no false change)")

    // (B) RECURSION GUARD — self-nesting existential comparison must TERMINATE (no shadow-stack overflow).
    let seed = Attribute(value: 1)
    let wrapped = Attribute(MakeWrapper(seed: seed, inner: leaf))  // any Shape = Wrapper(Leaf(5), seed 1)
    let wreader = Attribute(ReadShape(s: wrapped))
    check(wreader.value == 1005, "nested existential read: Wrapper(seed1, Leaf5) -> 1005")
    seed.value = 2
    check(wreader.value == 2005, "RECURSION GUARD: nested-existential change TERMINATES + propagates -> 2005")
    n.value = 7   // mutate the INNER leaf → the nested existential's inner changes through the chain
    check(wreader.value == 2007, "nested-existential INNER change propagates -> Wrapper(seed2, Leaf7) = 2007")

    // (C) churn the same existential compare many times (stress the recursion path / metadata stability).
    var problems = 0
    for i in 0..<500 {
        seed.value = i
        if wreader.value != i &* 1000 &+ 7 { problems += 1 }
    }
    check(problems == 0, "500x nested-existential re-compare stable (no metadata drift)")

    Subgraph.current = nil
    _ = graph
}
run()
print(failures == 0 ? "ALL OAG COMPARE TESTS PASSED" : "OAG COMPARE FAILURES: \(failures)")
