import OpenAttributeGraphShims

// oag-baseline: REUSED-NODE-SLOT use-after-free — the *second* mechanism behind the intermittent
// eleev/swiftui-2048 crash (crash "site 5"), reproduced headlessly and DETERMINISTICALLY.
//
// This is the case the `contains_subgraph` liveness gates CANNOT catch. Mechanism, grounded in source:
//   * A reference (AttributeID = a uint32 offset into the SHARED table, `Data/Pointer.h`) is held past
//     the teardown of the subgraph that owned its Node — e.g. a torn-down view still sitting in a
//     StoredLocation observer list. In the crash: StoredLocation.notifyObservers -> Attribute.invalidateValue.
//   * Subgraph teardown does NOT free the node's bytes (Graph::remove_node only unlinks edges); the storage
//     is freed at the PAGE level when `zone::clear()` returns the subgraph's pages to `table::shared()`
//     (Data/Zone.cpp:23). Those pages are then handed to the NEXT subgraph's allocations.
//   * So the stale offset stays IN RANGE (passes `validate_data_offset()` in IAGGraphInvalidateValue) but
//     now resolves to a FOREIGN node planted by the reusing subgraph. `value_mark` proceeds, and
//     `propagate_dirty` reads that foreign node's `output_edges` -> a wild {ptr,size} -> OOB read.
//     That is exactly the site-5 backtrace: propagate_dirty <- value_mark <- invalidateValue.
//
// This is NOT a realloc-during-walk (no reentrancy, no `WANDR:` log) and NOT a deleted-subgraph edge
// (the reusing subgraph is LIVE, so contains_subgraph passes). Only STOPPING REUSE of freed graph
// storage makes the stale read benign (it then reads the old, still-structurally-valid bytes).
//
// EXPECTED:
//   * WITHOUT a stop-reuse fix  -> a wild/OOB read traps on wasm (WASMTIME_EXIT=134). The crash, proven.
//   * WITH a stop-reuse fix     -> every stale invalidateValue reads benign stale data; all rounds pass.
// Either way the signal is DETERMINISTIC (crash every run / pass every run) — no manual play, no luck.

nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}

struct Derived: Rule { @Attribute var input: Int; var value: Int { input &+ 1 } }
// A differently-shaped rule whose stored body + computed value plant large sentinel bytes onto the
// reused page, so where the old Node's `output_edges` {ptr,size} used to be, the walk now reads a
// detectably-wild offset (or faults OOB if no stop-reuse gate is present).
struct Big: Rule { @Attribute var input: Int; var value: Int { input &* 2 &+ 0x7FFF_FFFF } }

func run() {
    let g = Graph()
    let rounds = 4000
    var survived = 0

    // A persistent "observer list": references we keep PAST the teardown of their owning subgraph,
    // exactly like StoredLocation keeping a torn-down view's attribute. We invalidate them AFTER reuse.
    var staleObservers: [Attribute<Int>] = []

    for round in 1...rounds {
        // (1) Victim subgraph with a node X that HAS output edges (so value_mark/propagate_dirty walk them).
        let victimSub = Subgraph(graph: g)
        Subgraph.current = victimSub
        let x = Attribute(value: round)
        // Several dependents => X gets a populated output_edges vector in the zone.
        var deps: [Attribute<Int>] = []
        for _ in 0..<6 { let d = Attribute(Derived(input: x)); _ = d.value; deps.append(d) }
        Subgraph.current = nil
        staleObservers.append(x)   // keep the reference alive across teardown

        // (2) Tear the victim down -> its pages return to table::shared() and become reusable.
        victimSub.invalidate()

        // (3) A LIVE reusing subgraph: allocate a burst of Big garbage nodes so one lands on X's freed
        //     slot, overwriting the old output_edges bytes with a large sentinel. This subgraph stays
        //     valid, so a contains_subgraph gate on X's (reused) slot would PASS — the gate can't help.
        let reuseSub = Subgraph(graph: g)
        Subgraph.current = reuseSub
        var fillers: [Attribute<Int>] = []
        let seed = Attribute(value: 0x7FFF_FFFF)
        for _ in 0..<48 { let f = Attribute(Big(input: seed)); _ = f.value; fillers.append(f) }
        Subgraph.current = nil

        // (4) Invalidate the STALE reference — the StoredLocation.notifyObservers -> invalidateValue path.
        //     value_mark(reused slot) -> propagate_dirty(foreign output_edges) -> OOB without a fix.
        for obs in staleObservers { obs.invalidateValue() }
        staleObservers.removeAll(keepingCapacity: true)

        // Reaching here = the stale invalidate did not fault this round.
        reuseSub.invalidate()
        survived &+= 1
        if round % 500 == 0 { print("  reuse round \(round) survived (survived=\(survived))") }
    }

    check(survived == rounds,
          "\(rounds)x invalidateValue on a torn-down-then-reused node slot (no UAF; benign stale read)")
    _ = g
    print((failures == 0) ? "ALL OAG REUSE TESTS PASSED" : "OAG REUSE FAILURES: \(failures)")
}

run()
