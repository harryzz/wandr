// WandrReactorExports — the reactor's WASI export surface, provided ONCE for any OpenSwiftUI-on-wandr
// app whose entry is its own unmodified `@main struct App`. An app links this library (and
// CWandrExports' component_type.o) instead of carrying its own per-app @_cdecl reactor stubs; the app
// dir then holds only the framework's own source.
//
// Each `@_cdecl` below is the implementation the wit-bindgen-c `__wasm_export_*` wrapper (in
// CWandrExports) calls by name; they unwrap the generated parameter types into plain values and
// forward straight into WandrRuntime. `on-init` boots the app's own @main via
// WandrRuntime.bootWandrReactorApp(). Opt-in: only apps that depend on this product get these symbols,
// so an app keeping its own hand-written stubs (e.g. OpenSwiftUIDemo) is unaffected.
import CWandrExports
import WandrRuntime

@_cdecl("exports_wandr_ui_shell_startup_on_init")
public func wandrReactorOnInit() {
    // Guaranteed-once host-driven entry (wandr:ui-shell/startup). Density is the reactor's only import
    // (wandr:ui-shell/metrics, via CWandrExports); the app's root view supplies everything visible, so
    // the clear color behind the DisplayList is a neutral default rather than a per-app constant.
    bootWandrReactorApp(
        background: CGColor(red: 0, green: 0, blue: 0, alpha: 1),
        getDensity: { wandr_ui_shell_metrics_get_density() }
    )
}

@_cdecl("exports_wasi_input_handlers_frame_handler_on_resize")
public func wandrReactorOnResize(_ w: UInt32, _ h: UInt32) { wandrRuntimeOnResize(width: w, height: h) }

@_cdecl("exports_wasi_input_handlers_frame_handler_on_frame")
public func wandrReactorOnFrame(_ nanos: UInt64) { wandrRuntimeOnFrame(nanos: nanos) }

@_cdecl("exports_wandr_ui_shell_frame_pacing_next_frame_delay")
public func wandrReactorNextFrameDelay() -> UInt32 { wandrRuntimeNextFrameDelay() }

@_cdecl("exports_wasi_input_handlers_pointer_handler_on_pointer")
public func wandrReactorOnPointer(_ ev: UnsafeMutablePointer<exports_wasi_input_handlers_pointer_handler_pointer_event_t>?) {
    guard let e = ev?.pointee else { return }
    let phase: Int
    switch e.kind {
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_DOWN): phase = 0
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_MOVE): phase = 1
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_UP): phase = 2
    default: phase = 3
    }
    wandrRuntimeOnPointer(phase: phase, x: Double(e.x), y: Double(e.y))
}

@_cdecl("exports_wandr_ui_shell_shell_events_on_scheduled_callback")
public func wandrReactorOnScheduledCallback(_ callbackId: UInt32) {}

@_cdecl("exports_wandr_ui_shell_shell_events_on_lifecycle_changed")
public func wandrReactorOnLifecycleChanged(_ newState: exports_wandr_ui_shell_shell_events_state_t) {
    // Drop any stale audio track across a background/foreground transition so the next play() opens
    // fresh rather than writing into a possibly-dead handle (matches wandr.audio.player).
    switch newState {
    case UInt8(WANDR_UI_SHELL_LIFECYCLE_STATE_PAUSED), UInt8(WANDR_UI_SHELL_LIFECYCLE_STATE_STOPPED),
         UInt8(WANDR_UI_SHELL_LIFECYCLE_STATE_RESUMED):
        WandrAudioPlayer.shared.reset()
    default: break
    }
}
