// [wandr Phase 2] UIKit stubs eleev uses for iPad-idiom checks + iOS List-background hacks. No-ops off-Apple.
import Foundation
import OpenSwiftUI
public enum UIUserInterfaceIdiom: Int, Sendable { case unspecified = -1, phone, pad, tv, carPlay, mac, vision }
public final class UIDevice: @unchecked Sendable {
    public static let current = UIDevice()
    public var userInterfaceIdiom: UIUserInterfaceIdiom { .phone }
}
// Minimal UIColor: an optional Color payload (nil == "no specific color", the old empty-stub
// behavior) plus the standard iOS system palette, so stock SwiftUI's `Color(.systemBlue)` etc.
// resolve. Immutable → @unchecked Sendable is sound.
public struct UIColor: @unchecked Sendable {
    public let color: Color?
    public init() { self.color = nil }
    public init(_ color: Color) { self.color = color }
    public static let clear = UIColor()
    public static let white = UIColor(Color(red: 1, green: 1, blue: 1, opacity: 1))
    public static let systemBlue  = UIColor(Color(red: 0.0,   green: 0.478, blue: 1.0,   opacity: 1.0))
    public static let systemGray2 = UIColor(Color(red: 0.682, green: 0.682, blue: 0.698, opacity: 1.0))
    public static let systemGray4 = UIColor(Color(red: 0.820, green: 0.820, blue: 0.839, opacity: 1.0))
}
public final class UITableView: @unchecked Sendable {
    nonisolated(unsafe) private static let shared = UITableView()
    public var backgroundColor: UIColor? = nil
    public static func appearance() -> UITableView { shared }
}
