import OpenAttributeGraphShims

// OAG zone/page allocator suite — PURE Compute/OAG. Exercises the wasm-divergent allocator: small-value
// recycle (≤0x10 alloc_bytes_recycle free-list), LARGE values (>0x10 alloc_bytes), and high-volume
// create→free→reuse to force page recycling and grow_region (on wasm a grow historically left OLD/NEW
// divergent copies; the page-seed mechanism rode here too). The baseline never stressed allocation.

nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}

struct MakeSmall: Rule { @Attribute var n: Int; var value: Int { n } }                 // ≤ 0x10
struct Big24 { var a: UInt64 = 0, b: UInt64 = 0, c: UInt64 = 0 }                        // 24B → alloc_bytes
struct MakeBig: Rule { @Attribute var n: Int; var value: Big24 { Big24(a: UInt64(n), b: UInt64(n &* 2), c: UInt64(n &* 3)) } }
struct ReadBig: Rule { @Attribute var v: Big24; var value: Int { Int(v.a &+ v.b &+ v.c) } }
struct MakeHuge: Rule { @Attribute var n: Int; var value: [Int] { Array(repeating: n, count: 64) } } // indirect/persistent

func run() {
    let graph = Graph()
    var problems = 0
    let rounds = 6000

    // Each round: a subgraph allocates small + large + huge values, verifies, then frees (invalidate).
    // The next round must safely RECYCLE the freed pages; volume forces grow_region.
    for round in 1...rounds {
        let sg = Subgraph(graph: graph)
        Subgraph.current = sg
        let n = Attribute(value: round)
        let small = Attribute(MakeSmall(n: n))
        let big = Attribute(MakeBig(n: n))
        let bigRead = Attribute(ReadBig(v: big))
        let huge = Attribute(MakeHuge(n: n))

        if small.value != round { problems += 1; if problems <= 3 { print("  MISMATCH small r\(round)") } }
        if bigRead.value != round &+ round &* 2 &+ round &* 3 { problems += 1 }
        if huge.value.count != 64 || huge.value.first != round { problems += 1 }

        // mutate → re-alloc/compare/destroy the large + huge values (recycle within a live zone)
        n.value = round &* 7
        if bigRead.value != round &* 7 &+ round &* 14 &+ round &* 21 { problems += 1 }

        Subgraph.current = nil
        sg.invalidate()                              // free pages → reusable for next round
        if round % 2000 == 0 { print("  memory round \(round) ok") }
    }

    if problems != 0 { failures += problems }
    check(failures == 0, "\(rounds) alloc(small+large+huge)/free/recycle rounds — pages recycle + grow safely")
    _ = graph
    print(failures == 0 ? "ALL OAG MEMORY TESTS PASSED (\(rounds) rounds)" : "OAG MEMORY FAILURES: \(failures)")
}
run()
