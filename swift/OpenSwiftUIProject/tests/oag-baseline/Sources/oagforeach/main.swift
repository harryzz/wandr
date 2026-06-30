import OpenAttributeGraphShims

// OAG dynamic-list (ForEach) reconstruction — PURE Compute/OAG, no OpenSwiftUI / OpenCoreGraphics.
//
// The eleev/swiftui-2048 demo freezes on wasm: the board renders frozen while the model advances.
// Root cause is NOT in OpenSwiftUI's ForEach — it is a Compute/OAG-layer property that ForEach is
// merely the first consumer of:
//
//   1. A `Subgraph` reference held inside a STRUCT field, across update cycles, must stay ALIVE
//      (on wasm `AG_BRIDGED_TYPE` is empty, so a struct-held foreign-ref Subgraph can go unmanaged
//      and its storage is freed while still referenced → use-after-free / stale reads).
//   2. ATTRIBUTE IDENTITY must be stable: a handle captured in a struct, re-read a cycle later,
//      must resolve the SAME attribute — not a stale duplicate. (The freeze = the render walks a
//      stale state while live data flows to a different instance: an identity split.)
//   3. VALUE CONSISTENCY: mutate an input/body, then a separately-held handle must read the new
//      value, not the value captured when the handle was first stored.
//
// This suite rebuilds the EXACT structural pattern ForEach relies on — a parent subgraph with
// dynamically added/removed CHILD subgraphs, each holding a per-item attribute, with every
// reference kept in a long-lived `ForEachState`-like struct across many reconcile→render cycles,
// and the allocator deliberately churned between update and render to force freed-storage reuse
// (the perturbation that turns a latent UAF into a hard failure). If Compute+OAG are correct this
// passes on linux AND wasm; if the foreign-ref Subgraph or attribute identity regresses, it fails
// HERE, at the right layer — so Compute/OAG can be blessed without dragging in the upper stack.

nonisolated(unsafe) var failures = 0
func check(_ cond: Bool, _ msg: String) {
    if cond { print("  ok  \(msg)") } else { print("  FAIL  \(msg)"); failures += 1 }
}

// A derived attribute so each "item" runs a rule inside its own child subgraph — exactly what a
// ForEach item's view body does (it isn't a bare value; it evaluates).
struct PlusBaseRule: Rule { @Attribute var x: Int; var value: Int { x &+ 1000 } }
struct DoubleRule: Rule { @Attribute var x: Int; var value: Int { x &* 2 } }

// MARK: - ForEachState reconstruction (mirrors ForEachState.items[id])

// One dynamic-list item: a child subgraph + a mutable input attribute + a derived output attribute.
// EVERY field that the freeze stresses is held here, by value, inside the long-lived `items` dict.
final class Item {
    let id: Int
    let child: Subgraph
    var input: Attribute<Int>
    var output: Attribute<Int>
    init(id: Int, child: Subgraph, input: Attribute<Int>, output: Attribute<Int>) {
        self.id = id; self.child = child; self.input = input; self.output = output
    }
}

final class ForEachStateLike {
    var items: [Int: Item] = [:]   // ← the struct-held child-subgraph refs, alive across all cycles
    let parent: Subgraph
    let graph: Graph
    init(parent: Subgraph, graph: Graph) { self.parent = parent; self.graph = graph }

    // Insert a brand-new item (ForEach .inserted path): new child subgraph under the parent.
    func insert(id: Int, value: Int) {
        let child = Subgraph(graph: graph)
        parent.addChild(child)
        let prev = Subgraph.current
        Subgraph.current = child
        let input = Attribute(value: value)
        let output = Attribute(PlusBaseRule(x: input))
        Subgraph.current = prev
        items[id] = Item(id: id, child: child, input: input, output: output)
    }

    // Update a surviving item's value in place (ForEach kept-item path) — the input attribute is
    // re-read via the struct-held handle, so a stale/dead handle shows up as a wrong render below.
    func update(id: Int, value: Int) {
        items[id]?.input.value = value
    }

    // Remove an item (ForEach .removed / eraseItem path): tear down + drop the child subgraph.
    func remove(id: Int) {
        guard let item = items[id] else { return }
        parent.removeChild(item.child)
        item.child.invalidate()
        items[id] = nil
    }

    // "Render": walk the items and read each derived output through the STRUCT-HELD handle. This is
    // the step that froze — it must observe live data, and every held child subgraph must be valid.
    func render() -> [Int: Int] {
        var out: [Int: Int] = [:]
        for (id, item) in items {
            // A struct-held child subgraph that went stale shows up as invalid here (counted, not spammed).
            if !item.child.isValid { print("  FAIL  item \(id) child subgraph INVALID at render"); failures += 1 }
            out[id] = item.output.value
        }
        return out
    }
}

// Reconcile the state to a target board (id -> tile value): update survivors, remove gone, insert new.
func reconcile(_ state: ForEachStateLike, to model: [Int: Int]) {
    for id in Array(state.items.keys) where model[id] == nil { state.remove(id: id) }
    for (id, value) in model {
        if state.items[id] != nil { state.update(id: id, value: value) }
        else { state.insert(id: id, value: value) }
    }
}

// Deliberately churn the allocator: create + invalidate throwaway subgraphs/attributes so any
// freed storage from a (wrongly) released struct-held Subgraph gets reused — turning a latent UAF
// into a deterministic wrong-value / invalid-subgraph failure. (The "perturbation" from RESUME.md.)
func churnAllocator(_ graph: Graph) {
    for k in 0..<8 {
        let sg = Subgraph(graph: graph)
        let prev = Subgraph.current
        Subgraph.current = sg
        let a = Attribute(value: k)
        let b = Attribute(DoubleRule(x: a))
        _ = b.value
        Subgraph.current = prev
        sg.invalidate()
    }
}

func run() {
    let graph = Graph()
    let parent = Subgraph(graph: graph)
    Subgraph.current = parent

    // ── Case 1: identity + value consistency of a STRUCT-HELD attribute handle across a mutation ──
    // (the precise "update mutates one instance, render must read it through a stored handle" split)
    struct Holder { var attr: Attribute<Int> }          // a struct that outlives the mutation
    let input1 = Attribute(value: 10)
    let derived1 = Attribute(DoubleRule(x: input1))
    let holder = Holder(attr: derived1)                  // store the handle in a struct, then churn
    check(holder.attr.value == 20, "case1: stored handle reads 20 (10*2)")
    churnAllocator(graph)
    input1.value = 50                                    // mutate the underlying input
    check(holder.attr.value == 100, "case1: stored handle re-reads LIVE value 100 (not frozen 20)")

    // ── Case 2: mutateBody then read through a separately-captured identity ──
    let input2 = Attribute(value: 3)
    let ruleAttr = Attribute(DoubleRule(x: input2))
    let storedId: AnyAttribute = ruleAttr.identifier     // capture identity in a local (struct field)
    check(ruleAttr.value == 6, "case2: rule reads 6 (3*2)")
    var bodyFired = false
    storedId.mutateBody(as: DoubleRule.self, invalidating: true) { body in
        bodyFired = true
        body = DoubleRule(x: Attribute(value: 21))           // rebind the rule's input
    }
    check(bodyFired, "case2: mutateBody callback fired")
    check(ruleAttr.value == 42, "case2: post-mutateBody read sees 42 (21*2), identity stable")

    // ── Case 3: the ForEach reconstruction — dynamic child-subgraph list across 40 churny cycles ──
    let state = ForEachStateLike(parent: parent, graph: graph)
    var model: [Int: Int] = [1: 2, 2: 2]                 // initial board: two tiles (like 2048 start)
    var nextId = 3
    reconcile(state, to: model)
    do {
        let rendered = state.render()
        let expected = model.mapValues { $0 &+ 1000 }
        check(rendered == expected, "cycle 0: rendered \(rendered.count) tiles match model")
    }

    for cycle in 1...40 {
        // advance the model deterministically: survivors merge-double, periodic removals, a spawn.
        for id in model.keys { model[id] = ((model[id]! &* 2) % 2048) &+ 2 }
        if cycle % 3 == 0, model.count > 1, let victim = model.keys.sorted().first { model[victim] = nil }
        model[nextId] = 2; nextId += 1
        if model.count > 12, let victim = model.keys.sorted().first { model[victim] = nil }

        reconcile(state, to: model)
        churnAllocator(graph)                            // ← force freed-storage reuse before render
        let rendered = state.render()
        let expected = model.mapValues { $0 &+ 1000 }
        check(rendered == expected,
              "cycle \(cycle): rendered board matches model (\(model.count) tiles)")
    }

    // tear down the surviving items
    for id in Array(state.items.keys) { state.remove(id: id) }
    Subgraph.current = nil
    _ = graph
}

run()
print(failures == 0 ? "ALL OAG FOREACH TESTS PASSED" : "OAG FOREACH FAILURES: \(failures)")
