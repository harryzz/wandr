// [wandr Phase 2] UIKit stubs eleev uses for iPad-idiom checks + iOS List-background hacks. No-ops off-Apple.
import Foundation
public enum UIUserInterfaceIdiom: Int, Sendable { case unspecified = -1, phone, pad, tv, carPlay, mac, vision }
public final class UIDevice: @unchecked Sendable {
    public static let current = UIDevice()
    public var userInterfaceIdiom: UIUserInterfaceIdiom { .phone }
}
public struct UIColor: Sendable { public static let clear = UIColor(); public static let white = UIColor(); public init() {} }
public final class UITableView: @unchecked Sendable {
    nonisolated(unsafe) private static let shared = UITableView()
    public var backgroundColor: UIColor? = nil
    public static func appearance() -> UITableView { shared }
}
