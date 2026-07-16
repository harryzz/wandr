// [wandr Phase 2] Real Combine bridge for NotificationCenter. OpenCombineFoundation (Apple's
// backing for .publisher(for:)) isn't wasm-buildable: it subscribes via the legacy
// `addObserver(forName:object:queue:using:)` API, which doesn't exist on wasm's
// `FoundationEssentials.NotificationCenter` at all (confirmed by dumping its swiftmodule symbols —
// `addObserver`/`post` there are ONLY the modern generic-Message overloads, no string-keyed API
// whatsoever). So we own both ends in-process instead: `post` writes into a per-name
// PassthroughSubject registry, `publisher(for:)` reads from the same registry. Single-threaded
// wasm — no locking needed. This previously returned an `Empty` publisher that never fired
// (confirmed: eleev's board-size setting change never reached GameLogic's Combine subscription,
// live or persisted) — a real bug, not a `@AppStorage`-covers-it fallback as the old comment
// assumed; eleev's `GameBoardSizeState` uses raw `UserDefaults` + `NotificationCenter`, not
// `@AppStorage`. Cross-process/host-level delivery is still a TODO (route via wandr:events) —
// this only wires up in-process (single wasm instance) delivery, which is everything eleev's own
// code needs.
import OpenCombine
import Foundation

public final class WandrNotificationBridge: @unchecked Sendable {
    public static let shared = WandrNotificationBridge()
    private var subjects: [Notification.Name: OpenCombine.PassthroughSubject<Notification, Never>] = [:]

    public func subject(for name: Notification.Name) -> OpenCombine.PassthroughSubject<Notification, Never> {
        if let existing = subjects[name] { return existing }
        let subject = OpenCombine.PassthroughSubject<Notification, Never>()
        subjects[name] = subject
        return subject
    }
}

public extension NotificationCenter {
    func post(name: Notification.Name, object: Any?, userInfo: [AnyHashable: Any]?) {
        WandrNotificationBridge.shared.subject(for: name).send(Notification(name: name, object: object, userInfo: userInfo))
    }

    func publisher(for name: Notification.Name, object: AnyObject? = nil) -> OpenCombine.PassthroughSubject<Notification, Never> {
        WandrNotificationBridge.shared.subject(for: name)
    }
}
