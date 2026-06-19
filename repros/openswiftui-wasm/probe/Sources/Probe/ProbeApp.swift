import OpenSwiftUI
// ISOLATE: VStack/TupleView/layout alone (no @State, no Text). Conformance shim cleared
// the swift_conformsToProtocol trap; does multi-child layout itself render or hang?
@main
struct ProbeApp: App {
    var body: some Scene {
        WindowGroup {
            VStack(spacing: 10) { Color.red; Color.blue }
        }
    }
}
