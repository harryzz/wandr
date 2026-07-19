// [wandr] Real SwiftUI re-exports Combine, so `import SwiftUI` alone brings `ObservableObject` /
// `@Published` into scope (the MVVM view-models that @StateObject/@ObservedObject bind to). Our
// SwiftUI shim must do the same, or a stock file that declares `class VM: ObservableObject` with
// only `import SwiftUI` won't compile. (Combine here → OpenCombine, via the sibling Combine shim.)
@_exported import Combine
