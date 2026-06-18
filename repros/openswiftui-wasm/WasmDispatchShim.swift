// WasmDispatchShim.swift — minimal single-threaded Dispatch for wasm (no libdispatch).
//
// wasm has no GCD. OpenSwiftUICore uses a tiny Dispatch surface on the scheduling/
// animation path (DisplayListViewRenderer, CAHostingLayer, AnimationListener):
// `DispatchQueue.main.async`/`.asyncAfter(deadline:)` + `DispatchTime`. This provides
// just that surface. **Compile-focused**: closures run synchronously and `now()` is a
// monotonic stub for now; real deferral/timing gets wired to the wandr frame loop in a
// later phase (the renderer host drives `on_frame`).
#if os(WASI)

public struct DispatchTime: Comparable {
    public var uptimeNanoseconds: UInt64
    public init(uptimeNanoseconds: UInt64) { self.uptimeNanoseconds = uptimeNanoseconds }

    public static func now() -> DispatchTime {
        // TODO(phase 3+): drive from the host monotonic clock / frame nanos.
        DispatchTime(uptimeNanoseconds: 0)
    }
    public static func < (a: DispatchTime, b: DispatchTime) -> Bool {
        a.uptimeNanoseconds < b.uptimeNanoseconds
    }
    /// Dispatch's `DispatchTime + seconds` overload.
    public static func + (t: DispatchTime, seconds: Double) -> DispatchTime {
        DispatchTime(uptimeNanoseconds: t.uptimeNanoseconds &+ UInt64(max(0, seconds) * 1_000_000_000))
    }
}

public final class DispatchQueue {
    public static let main = DispatchQueue()
    // Single-threaded: run now. (A frame-loop-backed pending queue replaces this later.)
    public func async(execute work: () -> Void) { work() }
    public func asyncAfter(deadline: DispatchTime, execute work: () -> Void) { work() }
}

#endif
