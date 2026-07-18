// [wandr Phase 2 — Stage 1] Thin reactor export stubs for T2iles.
//
// eleev's own `@main struct T2ilesApp: App` (T2ilesApp.swift) is the app entry now, used VERBATIM —
// `on-init` below arms the wandr reactor and runs that @main via WandrRuntime.bootWandrReactorApp(),
// which registers the app and returns so the host can drive frames. These `@_cdecl` stubs are the
// last per-app glue: each unwraps its wit-bindgen-generated parameter type and forwards straight into
// WandrRuntime. Stage 2 moves them into the shared CWandrShell leaf so the app carries ZERO wandr code.
import CSwiftSpike
import WandrRuntime

// TOUCHES wasi:canvas (transitively, via WandrRuntime); the -DWANDR_HEADLESS deterministic driver has
// no canvas, so gate it out (it supplies its own inert export stubs — see WandrHeadless.swift).
#if !WANDR_HEADLESS

@_cdecl("exports_wandr_ui_shell_startup_on_init")
public func onInit() {
    // Guaranteed-once host-driven entry (wandr:ui-shell/startup). Boots eleev's own @main app; the
    // density query is the app's only remaining WIT import (wandr:ui-shell/metrics via CSwiftSpike).
    bootWandrReactorApp(
        background: CGColor(red: 0.063, green: 0.078, blue: 0.094, alpha: 1),
        getDensity: { wandr_ui_shell_metrics_get_density() }
    )
}

@_cdecl("exports_wasi_input_handlers_frame_handler_on_resize")
public func onResize(_ w: UInt32, _ h: UInt32) { wandrRuntimeOnResize(width: w, height: h) }

@_cdecl("exports_wasi_input_handlers_frame_handler_on_frame")
public func onFrame(_ nanos: UInt64) { wandrRuntimeOnFrame(nanos: nanos) }

@_cdecl("exports_wandr_ui_shell_frame_pacing_next_frame_delay")
public func nextFrameDelay() -> UInt32 { wandrRuntimeNextFrameDelay() }

@_cdecl("exports_wasi_input_handlers_pointer_handler_on_pointer")
public func onPointer(_ ev: UnsafeMutablePointer<exports_wasi_input_handlers_pointer_handler_pointer_event_t>?) {
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
public func onScheduledCallback(_ callbackId: UInt32) {}

@_cdecl("exports_wandr_ui_shell_shell_events_on_lifecycle_changed")
public func onLifecycleChanged(_ newState: exports_wandr_ui_shell_shell_events_state_t) {
    // A track can go stale across a background/foreground transition (the OS may reclaim the AAudio
    // stream while backgrounded, or the granted ring depth differs by role); drop on the transition
    // either direction so the next play() opens fresh rather than writing into a possibly-dead handle.
    switch newState {
    case UInt8(WANDR_UI_SHELL_LIFECYCLE_STATE_PAUSED), UInt8(WANDR_UI_SHELL_LIFECYCLE_STATE_STOPPED),
         UInt8(WANDR_UI_SHELL_LIFECYCLE_STATE_RESUMED):
        WandrAudioPlayer.shared.reset()
    default: break
    }
}
#endif // !WANDR_HEADLESS
