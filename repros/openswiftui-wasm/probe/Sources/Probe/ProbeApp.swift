import OpenSwiftUI

// ✅ Text + @State render on wasm (clean exit). @State is stored, read in body, and
// drives the DisplayList via the reactive graph (onUpdate/onInvalidation now wired as
// stored plain-C callbacks). The stdout renderer doesn't emit .text glyphs (only fills),
// so a Text body shows an empty "rendered:"; the @State→Color.opacity variant shows the
// state value visibly (alpha 0x40 = 0.25 for count>5). Real text = phase-4 CGContext drawer.
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
