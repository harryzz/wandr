// [wandr Phase 2] Foundation bits missing/limited on wasm.
// - OperationQueue → synchronous stub (single-threaded wasm).
// - NotificationCenter.post lacks the (name:object:userInfo:) overload on wasm Foundation; add it.
//   Body is a no-op: eleev's only listener is the settings board-size change, which also flows via
//   @AppStorage — live NotificationCenter propagation is a TODO.
import Foundation
public final class OperationQueue: @unchecked Sendable {
    public static let main = OperationQueue()
    public init() {}
    public func addOperation(_ block: @escaping () -> Void) { block() }
}
public extension NotificationCenter {
    func post(name: Notification.Name, object: Any?, userInfo: [AnyHashable: Any]?) {
        wandrShimWarnOnce("shim: NotificationCenter.post NO-OP (no live listeners; board-size uses @AppStorage)")
        // TODO: real post (route to wandr:events?). Board-size change also uses @AppStorage.
    }
}
