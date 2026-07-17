// [wandr Phase 2] Reactor entry for the REAL eleev/swiftui-2048 app. Replaces eleev's
// @main T2ilesApp (excluded — it's the platform-entry seam, like Audio/Store): builds
// eleev's CompositeView and drives it through the shared WandrRuntime reactor loop. Eleev's
// own Sources compile UNMODIFIED behind the SwiftUI/Combine/AudioToolbox shims.
//
// Per Sources/T2iles/RULES.md, this file carries ONLY Audio/Store/startup: which board size
// to boot with, when to pump queued audio, and audio-driven frame pacing. Everything else (the
// WandrDrawSink conformer, the wasi:canvas handshake, frame pacing, pointer forwarding) lives in
// WandrRuntime — see that package's own doc comment for why four tiny forwarding stubs are the
// one thing that CAN'T move there (each app's exported-function parameter types are generated
// fresh from its own wit world).
import CSwiftSpike
import WandrRuntime
import OpenSwiftUI
#if canImport(WASILibc)
import WASILibc
#endif
@inline(never) func wlog(_ s: String) { fputs("T2ILES: \(s)\n", stderr); fflush(stderr) }

nonisolated(unsafe) var sharedGame: GameLogic!

struct WandrHostApp: App {
    var body: some Scene { WindowGroup { CompositeView(board: sharedGame) } }
}

// [wandr] Everything below TOUCHES wasi:canvas (transitively, via WandrRuntime). The
// -DWANDR_HEADLESS deterministic driver has no canvas, so gate it out — otherwise the module
// still imports wasi:canvas and wasmtime refuses to instantiate the plain command.
#if !WANDR_HEADLESS

private func bootRuntime() {
    runWandrApp(
        background: CGColor(red: 0.063, green: 0.078, blue: 0.094, alpha: 1),
        getDensity: { wandr_ui_shell_metrics_get_density() },
        willRenderFrame: { _, _ in
            if sharedGame == nil {
                // [wandr] Mirror eleev's own T2ilesApp.mainView (excluded from this build —
                // WandrReactor replaces it as the platform entry): read the board size the user
                // picked in Settings by asking eleev's OWN GameBoardSizeState — it already knows
                // how to read+fall back, no need to re-derive that logic here.
                let boardSize = GameBoardSizeState().boardSize
                sharedGame = GameLogic(size: boardSize.rawValue)
            }
            // Drain any audio still queued from a recent play() — a single write() only fills
            // what currently fits the ring (confirmed on-device: ~19% of a short SFX in one
            // call), so the remainder is pumped in a bit more each frame as the device consumes it.
            WandrAudioPlayer.shared.pump()
        },
        isBusy: { WandrAudioPlayer.shared.isActive },
        makeApp: { WandrHostApp() }
    )
}
nonisolated(unsafe) private var runtimeBooted = false

@_cdecl("exports_wasi_input_handlers_frame_handler_on_resize")
public func onResize(_ w: UInt32, _ h: UInt32) {
    wandrRuntimeOnResize(width: w, height: h)
}

@_cdecl("exports_wasi_input_handlers_frame_handler_on_frame")
public func onFrame(_ nanos: UInt64) {
    if !runtimeBooted { bootRuntime(); runtimeBooted = true }
    wandrRuntimeOnFrame(nanos: nanos)
}

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
    // A track can go stale across a background/foreground transition (the OS may reclaim the
    // AAudio stream while backgrounded, or the ring depth the host grants differs by role) —
    // matches wandr.audio.player's reopen_at_current. Drop on the transition either direction so
    // the next play() call always opens fresh rather than writing into a possibly-dead handle.
    switch newState {
    case UInt8(WANDR_UI_SHELL_LIFECYCLE_STATE_PAUSED), UInt8(WANDR_UI_SHELL_LIFECYCLE_STATE_STOPPED),
         UInt8(WANDR_UI_SHELL_LIFECYCLE_STATE_RESUMED):
        WandrAudioPlayer.shared.reset()
    default: break
    }
}
#endif // !WANDR_HEADLESS
