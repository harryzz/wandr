import OpenSwiftUI

@main
struct ProbeApp: App {
    // Minimal: a single Color (no VStack/TupleView) exercises the fewest swiftcall
    // callbacks — aims to reach a rendered DisplayList with walls already cleared,
    // sidestepping the TupleView→conformance path.
    var body: some Scene {
        WindowGroup {
            Color.red
        }
    }
}
