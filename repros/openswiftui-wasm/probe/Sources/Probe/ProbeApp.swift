import OpenSwiftUI

// TARGET (next): custom View with @State + Text. Currently BLOCKED — reading @State
// hangs (reactive update/invalidation never settles) because onUpdate/onInvalidation
// are no-op'd on wasm. Fix = wire them as proper stored plain-C callbacks. Custom View
// WITHOUT @State already renders; interfaceIdiom + Bundle.main walls are cleared.
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
