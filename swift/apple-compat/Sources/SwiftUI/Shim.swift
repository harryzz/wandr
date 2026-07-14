// [wandr Phase 2] `SwiftUI` shim: makes eleev's real `import SwiftUI` resolve to OpenSwiftUI,
// unmodified. Re-exports OpenSwiftUI; platform-only pieces the app needs but OpenSwiftUI lacks
// (@AppStorage = Store, Image(systemName:) = SF symbols) get added here incrementally.
@_exported import OpenSwiftUI
// Apple's `import SwiftUI` also surfaces Foundation/CoreGraphics types (CGFloat, CGSize,
// CGRect, CGPoint, URL, UserDefaults, Notification…). Re-export Foundation so eleev's real
// sources see them unmodified (on wasm, swift-corelibs-Foundation carries the CG geometry).
@_exported import Foundation
