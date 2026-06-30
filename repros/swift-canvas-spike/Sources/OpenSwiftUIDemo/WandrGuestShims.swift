//  WandrGuestShims.swift — guest-layer shims for Foundation concurrency/scheduling primitives
//  absent on wasip1 (single-threaded). The wandr guest is wasm-only, so these run synchronously.
//  Kept OUT of eleev/ (eleev's sources stay verbatim) and OUT of OpenSwiftUI core (this is the
//  app/platform layer).
import Foundation

final class OperationQueue {
    nonisolated(unsafe) static let main = OperationQueue()
    func addOperation(_ block: @escaping () -> Void) { block() }
}

import OpenCombine

// wandr guest shim: eleev subscribes to a board-resize Notification via Combine's
// NotificationCenter.publisher(for:). We don't support runtime board resize, so make it an
// inert (never-emitting) OpenCombine publisher — the .map/.sink/.store chain still compiles.
// A no-op publisher with its own .map/.sink so eleev's board-resize subscription compiles
// WITHOUT building an OpenCombine operator chain (OpenCombine.FilterProducer's generic
// CustomStringConvertible witness faults on aarch64 AOT). The listener is inert (no resize).
struct _WandrDeadPublisher<Output> {
    func map<T>(_ transform: @escaping (Output) -> T) -> _WandrDeadPublisher<T> { _WandrDeadPublisher<T>() }
    func sink(receiveValue: @escaping (Output) -> Void) -> AnyCancellable { AnyCancellable {} }
}
extension NotificationCenter {
    func publisher(for name: Notification.Name, object: AnyObject? = nil) -> _WandrDeadPublisher<Notification> {
        _WandrDeadPublisher<Notification>()
    }
}
