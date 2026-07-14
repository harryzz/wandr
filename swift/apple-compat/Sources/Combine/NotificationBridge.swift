// [wandr Phase 2] Minimal Combine bridge for NotificationCenter.publisher(for:), which lives in
// OpenCombineFoundation (not wasm-buildable). Returns an Empty publisher — our NotificationCenter
// post() is a no-op, so notification-driven chains compile + run but never fire (board-size change
// also flows via @AppStorage). TODO: real events via wandr:events.
import OpenCombine
import Foundation

public extension NotificationCenter {
    func publisher(for name: Notification.Name, object: AnyObject? = nil) -> OpenCombine.Empty<Notification, Never> {
        OpenCombine.Empty<Notification, Never>(completeImmediately: false)
    }
}
