// [wandr Phase 2] Foundation test: does eleev-style `import SwiftUI` + `import Combine` compile
// an ObservableObject view through the shims, with ZERO OpenSwiftUI/OpenCombine references?
import SwiftUI
import Combine

final class Model: ObservableObject {
    @Published var n: Int = 0
}
struct ContentView: View {
    @ObservedObject var model: Model
    @State private var flag = false
    var body: some View {
        VStack {
            Text("n = \(model.n)")
            Text(flag ? "on" : "off")
        }
    }
}
print("shim-test: import SwiftUI + import Combine compiled an ObservableObject View")
