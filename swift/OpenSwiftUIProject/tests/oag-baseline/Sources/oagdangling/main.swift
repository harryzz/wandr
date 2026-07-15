import OpenAttributeGraphShims

// oag-baseline: REENTRANT-mutation-during-walk UAF — the true root of the intermittent eleev/swiftui-2048
// AttributeGraph crash, reproduced headlessly (no OpenSwiftUI, no views, no rendering).
//
// MECHANISM (crash "site 1"). `Graph::propagate_dirty` (Compute Graph.cpp) snapshots a RAW pointer
// {&Node::output_edges().front(), size} onto its C++ frame stack and iterates it. The ONLY thing that
// runs graph-mutating Swift code SYNCHRONOUSLY inside that walk is the invalidation callback
// (call_invalidation_if_needed → onInvalidation), and it fires ONLY across a CONTEXT boundary
// (context_id != the propagating attribute's context). If that callback grows the walked node's
// output_edges (add_output_edge → data::vector::push_back → zone::realloc_bytes MOVE), the old buffer is
// recycled and the next allocation overwrites the bytes the walk still points at → wild pointer.
//
// WHY THE STRAIGHT-LINE VERSION PASSED: it used a single Graph(), so every subgraph shared ONE context,
// the cross-context invalidation callback was never armed, no reentrant mutation happened, and the
// dangling edge was benign. The missing ingredient is TWO contexts over one graph: `Graph(shared:)`.
//
// EXPECTED: on a build WITH the g_iag_graph_walk_depth guard (Zone.cpp — retain reallocated buffers
// during a walk) this PASSES and prints reallocFired>0 (the reentrancy really happens, now survived).
// With that guard removed/#if 0'd, it reads a wild pointer on wasm and traps → the crash, proven.

nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}

struct Derived: Rule { @Attribute var input: Int; var value: Int { input &+ 1 } }
struct Big: Rule { @Attribute var input: Int; var value: Int { input &+ 0x7FFF_FFFF } }

// Shared with the invalidation callback (single-threaded wasm/linux — plain globals are fine).
nonisolated(unsafe) var currentA: Attribute<Int>? = nil
nonisolated(unsafe) var parentSubgraph: Subgraph? = nil
nonisolated(unsafe) var reentering = false
nonisolated(unsafe) var reallocFired = 0
nonisolated(unsafe) var sink = 0

func run() {
    // TWO CONTEXTS over ONE underlying graph — the missing ingredient that arms the invalidation callback.
    let gA = Graph()
    let gB = Graph(shared: gA)
    let pA = Subgraph(graph: gA)   // context A — owns `a` and its same-context dependents
    parentSubgraph = pA

    // Fires SYNCHRONOUSLY from inside propagate_dirty when a cross-context dependent is dirtied.
    gB.onInvalidation { _ in
        guard !reentering, let a = currentA, let pA = parentSubgraph else { return }
        reentering = true
        defer { reentering = false }
        reallocFired &+= 1
        // Grow a.output_edges WHILE propagate_dirty holds a raw pointer into it → realloc MOVE.
        Subgraph.current = pA
        for _ in 0..<8 { let extra = Attribute(Derived(input: a)); sink &+= extra.value }
        // Overwrite the just-freed old buffer with a LARGE sentinel so the resumed walk reads a
        // detectably-wild output-edge offset (or an OOB deref if the guard that skips wild offsets is off).
        for _ in 0..<8 { let filler = Attribute(Big(input: a)); sink &+= filler.value }
        Subgraph.current = nil
    }

    let rounds = 2000
    var mismatches = 0

    for round in 1...rounds {
        Subgraph.current = pA
        let a = Attribute(value: round)
        currentA = a

        // A same-context dependent created BEFORE the cross edge → in the reverse walk it is visited
        // AFTER the cross edge, so a live edge remains to be dereferenced from the (now stale) buffer.
        let c0 = Attribute(Derived(input: a))
        _ = c0.value

        // CROSS-CONTEXT dependent chain in context B. bCross must itself have an output edge (dCross)
        // or propagate_dirty skips the invalidation-callback path.
        var pB = Subgraph(graph: gB)
        pA.addChild(pB)
        Subgraph.current = pB
        let bCross = Attribute(Derived(input: a))
        let dCross = Attribute(Derived(input: bCross))
        if bCross.value != round &+ 1 { mismatches &+= 1 }
        if dCross.value != round &+ 2 { mismatches &+= 1 }

        // More same-context dependents AFTER the cross edge: pad a.output_edges toward a realloc
        // boundary and keep the buffer off the page tail (so growth MOVES the buffer, not in-place).
        Subgraph.current = pA
        var tail: [Attribute<Int>] = []
        for _ in 0..<6 { let cn = Attribute(Derived(input: a)); _ = cn.value; tail.append(cn) }

        // DIRTY a → mark_changed → propagate_dirty walks a.output_edges, dirties the cross-context
        // bCross, fires onInvalidation REENTRANTLY, which reallocs a.output_edges mid-walk.
        // THE CRASH POINT on a guard-less build.
        a.value = round &* 10

        // Reaching here = the walk survived the reentrant realloc. Surviving dependents must recompute.
        if c0.value != round &* 10 &+ 1 { mismatches &+= 1 }
        for cn in tail where cn.value != round &* 10 &+ 1 { mismatches &+= 1 }

        // Tear down the cross-context child subgraph (frees bCross/dCross on wasm) like eraseItem.
        Subgraph.current = pA
        pA.removeChild(pB)
        pB.invalidate()

        Subgraph.current = nil
        currentA = nil
        if round % 500 == 0 { print("  reentrant-realloc round \(round) survived (reallocFired=\(reallocFired))") }
    }

    check(reallocFired > 0,
          "invalidation callback fired reentrantly during propagate_dirty (reallocFired=\(reallocFired)) — the reentrancy is exercised")
    check(mismatches == 0,
          "\(rounds)x reentrant add_output_edge during propagate_dirty over a cross-context edge (no UAF; dependents correct)")
    _ = (gA, gB)
    print((failures == 0) ? "ALL OAG REENTRANT-WALK TESTS PASSED" : "OAG REENTRANT-WALK FAILURES: \(failures)")
}

run()
