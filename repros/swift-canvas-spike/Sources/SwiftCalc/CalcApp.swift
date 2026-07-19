// [wandr portability test] The ONLY added file: a standard SwiftUI @main entry, replacing the
// UIKit AppDelegate/SceneDelegate the original used (iOS-13 lifecycle). The 9 view files
// (ContentView, DisplayView, NumberPad, ...) are andreiui/swift-calculator VERBATIM — the whole
// point is to see how much pure SwiftUI builds on OpenSwiftUI-on-wandr with zero view edits.
import SwiftUI

@main
struct CalcApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}
