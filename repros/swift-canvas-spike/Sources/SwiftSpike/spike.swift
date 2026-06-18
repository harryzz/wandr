// Task 114 P1 — the Swift side of the custom-WIT round trip.
// Imports `wandr:swift-spike/host` (log, draw-rect) and exports `run`, both via
// Swift's first-class C interop over the wit-bindgen-c surface (generated/).
// Ready to compile with the Swift wasm32-unknown-wasi SDK (see build.sh); not yet
// built here (no Swift toolchain in this environment).
import CSwiftSpike

// Export the WIT `run`: wit-bindgen-c declares `exports_swift_spike_run`; we
// satisfy it with @_cdecl so the component's export resolves to this Swift fn.
@_cdecl("exports_swift_spike_run")
public func swiftSpikeRun() {
    hostLog("hello from swift -> wasi (custom-WIT round trip)")
    // A few rects — the stand-in for wasi:canvas draw-rect (argb 0xAARRGGBB).
    wandr_swift_spike_host_draw_rect(10, 10, 120, 48, 0xFF3366CC)
    wandr_swift_spike_host_draw_rect(24, 70, 96, 96, 0xFFCC6633)
    wandr_swift_spike_host_draw_rect(140, 10, 48, 156, 0xFF33AA55)
}

private func hostLog(_ s: String) {
    s.withCString { c in
        var ws = swift_spike_string_t(ptr: nil, len: 0)
        swift_spike_string_set(&ws, c)
        wandr_swift_spike_host_log(&ws)
    }
}
