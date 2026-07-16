// [wandr Phase 2] Foundation bits missing/limited on wasm.
// - OperationQueue → synchronous stub (single-threaded wasm).
// - NotificationCenter.post(name:object:userInfo:) → see Combine/NotificationBridge.swift, which
//   owns the real post+publisher pairing (wasm's FoundationEssentials.NotificationCenter has no
//   string-keyed API at all to delegate to — it's generic-Message-only).
import Foundation
public final class OperationQueue: @unchecked Sendable {
    public static let main = OperationQueue()
    public init() {}
    public func addOperation(_ block: @escaping () -> Void) { block() }
}
