import OpenAttributeGraphShims

// OAG Swift-ABI bridge suite — PURE Compute/OAG. The DEEPEST wasm divergence + the 11-day-regression
// root: `Subgraph` is a foreign-reference (CF/objc_bridge) type, but `objc_bridge(id)` is EMPTY on wasm,
// so a `Subgraph` reference held in a STRUCT/CLASS field is ARC-UNMANAGED → its storage can be freed
// while still referenced → use-after-free (the ForEachState/DynamicViewList immortal-storage family).
// Also covers Subgraph.apply / forEach / isValid / isIdentical (identity stability). UNTESTED by baseline.

nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}

// A long-lived holder keeping foreign-ref Subgraph refs (+ their attributes) across ALL cycles.
final class Holder { var children: [Subgraph] = []; var attrs: [Attribute<Int>] = [] }

func run() {
    let graph = Graph()
    let parent = Subgraph(graph: graph)
    let holder = Holder()
    var problems = 0

    // Build children, stash their Subgraph refs by-value in `holder`, churn the allocator hard between
    // cycles (throwaway create+invalidate to reuse freed storage), then re-read EVERY held attr through
    // its held subgraph ref. If the foreign-ref Subgraph storage is freed-while-referenced, this faults.
    for cycle in 1...60 {
        Subgraph.current = parent
        let child = Subgraph(graph: graph); parent.addChild(child)
        Subgraph.current = child
        let attr = Attribute(value: cycle)
        holder.children.append(child)        // struct-held foreign-ref Subgraph, alive across cycles
        holder.attrs.append(attr)
        Subgraph.current = nil

        for _ in 0..<30 {                    // churn freed storage
            let t = Subgraph(graph: graph); parent.addChild(t); parent.removeChild(t); t.invalidate()
        }

        for (i, a) in holder.attrs.enumerated() {
            if a.value != i + 1 { problems += 1; if problems <= 3 { print("  STALE attr[\(i)] = \(a.value) != \(i+1)") } }
            if !holder.children[i].isValid { problems += 1; if problems <= 3 { print("  DEAD held subgraph[\(i)]") } }
        }
    }
    check(problems == 0, "60 cycles × struct-held foreign-ref Subgraph survive allocator churn (no UAF / no identity split)")

    // Subgraph.apply / isValid / isIdentical / invalidate
    Subgraph.current = parent
    let c1 = Subgraph(graph: graph); parent.addChild(c1)
    let inApply = c1.apply { Attribute(value: 99) }
    check(inApply.value == 99, "Subgraph.apply runs closure with that subgraph current")
    check(c1.isValid, "isValid true for live subgraph")
    check(c1.isIdentical(to: c1) && !c1.isIdentical(to: parent), "Subgraph identity (isIdentical) distinguishes")
    c1.invalidate()
    check(!c1.isValid, "isValid false after invalidate")

    Subgraph.current = nil
    if problems != 0 { failures += 0 }   // already counted via check
    _ = graph
    print(failures == 0 ? "ALL OAG BRIDGE TESTS PASSED" : "OAG BRIDGE FAILURES: \(failures)")
}
run()
