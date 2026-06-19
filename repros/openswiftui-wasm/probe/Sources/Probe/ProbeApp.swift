import OpenSwiftUI
// WORKING on wasm: custom View + @State (reactive) + Text. Multi-child (VStack/TupleView)
// is blocked on MEMORY CORRUPTION in the TupleView/conformance path (see RESUME) — the
// failure point moves with any code change, so print-instrumentation can't localize it.
struct ContentView: View {
    @State private var count = 7
    var body: some View { Text("count \(count)") }
}
@main
struct ProbeApp: App {
    var body: some Scene { WindowGroup { ContentView() } }
}
