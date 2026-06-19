import OpenSwiftUI

// WORKING on wasm: custom View + @State (reactive) + Text. (VStack/multi-child is
// blocked on the TupleView per-element runtime-conformance witness dispatch — see RESUME.)
struct ContentView: View {
    @State private var count = 7
    var body: some View {
        Text("count \(count)")
    }
}

@main
struct ProbeApp: App {
    var body: some Scene { WindowGroup { ContentView() } }
}
