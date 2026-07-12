// [wandr Phase 2] `SwiftUI` shim: makes eleev's real `import SwiftUI` resolve to OpenSwiftUI,
// unmodified. Re-exports OpenSwiftUI; platform-only pieces the app needs but OpenSwiftUI lacks
// (@AppStorage = Store, Image(systemName:) = SF symbols) get added here incrementally.
@_exported import OpenSwiftUI
