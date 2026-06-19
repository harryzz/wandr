import OpenSwiftUI
// MULTI-CHILD WALL CLEARED (2026-06-19): VStack { Color.red; Color.blue } => TupleView<(Color,Color)>
// renders TWO fills (red top, blue bottom, 8pt VStack gap) and exits 0, deterministically.
// The wall was NOT memory corruption / TupleView witness tables (the RESUME's prior hypothesis)
// — it was the AGGraphReadCachedAttribute swiftcc closure-with-arg/return signature mismatch hit
// by the LAYOUT engine on multi-child views. Diagnosed non-invasively via wasmtime -D coredump +
// DWARF (frame 0 = signature_mismatch:AGGraphReadCachedAttribute). Fixed with the plain-C
// AGGraphReadCachedAttributeC *C variant (compute-wasm.patch). @State + Text on the multi-child
// path also run to exit 0 (Text emits no fill in the stdout renderer; glyphs = phase-4 drawer).
struct ContentView: View {
    var body: some View {
        VStack {
            Color.red
            Color.blue
        }
    }
}
@main
struct ProbeApp: App {
    var body: some Scene { WindowGroup { ContentView() } }
}
