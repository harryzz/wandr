// [wandr Phase 2] Reactor entry for the REAL eleev/swiftui-2048 app. Replaces eleev's
// @main T2ilesApp (excluded — it's the platform-entry seam, like Audio/Store): builds
// eleev's CompositeView and drives it through wasi:canvas via the WandrRenderer SPI +
// the wasi:input-handlers reactor exports. Eleev's own Sources compile UNMODIFIED behind
// the SwiftUI/Combine/AudioToolbox shims.
import CSwiftSpike
import WandrCG
import OpenSwiftUI
@_spi(WandrRenderer) import OpenSwiftUI
#if canImport(WASILibc)
import WASILibc
#endif
@inline(never) func wlog(_ s: String) { fputs("T2ILES: \(s)\n", stderr); fflush(stderr) }

nonisolated(unsafe) var sharedGame: GameLogic!

struct WandrHostApp: App {
    var body: some Scene { WindowGroup { CompositeView(board: sharedGame) } }
}

// MARK: - WandrDrawSink over a CGContext (OpenSwiftUI DisplayList → CoreGraphics → wasi:canvas)
final class CGSink: WandrDrawSink {
    nonisolated(unsafe) var cg: CGContext?
    func beginFrame(width: Double, height: Double, version: UInt32) {}
    func fillRect(x: Double, y: Double, width: Double, height: Double,
                  red: Float, green: Float, blue: Float, opacity: Float) {
        guard let cg else { return }
        cg.setFillColor(CGColor(red: CGFloat(red), green: CGFloat(green), blue: CGFloat(blue), alpha: CGFloat(opacity)))
        cg.fill(CGRect(x: CGFloat(x), y: CGFloat(y), width: CGFloat(width), height: CGFloat(height)))
    }
    func drawText(_ text: String, x: Double, y: Double, width: Double, height: Double,
                  fontSize: Double, red: Float, green: Float, blue: Float, opacity: Float) {
        guard let cg else { return }
        cg.drawString(text, at: CGPoint(x: CGFloat(x), y: CGFloat(y)), size: CGFloat(fontSize),
                      color: CGColor(red: CGFloat(red), green: CGFloat(green), blue: CGFloat(blue), alpha: CGFloat(opacity)),
                      maxWidth: CGFloat(width))
    }
    func endFrame() {}
}

// MARK: - Reactor state (wasm32-wasip1 single-threaded ⇒ globals are safe)
nonisolated(unsafe) private let sink = CGSink()
nonisolated(unsafe) private var width: Float = 0
nonisolated(unsafe) private var height: Float = 0
nonisolated(unsafe) private var built = false
nonisolated(unsafe) private var animPending = false
nonisolated(unsafe) private var lastFrameNanos: UInt64 = 0
nonisolated(unsafe) private var pointerPressing = false
nonisolated(unsafe) private var ptrSerial = 0

@_cdecl("exports_wasi_input_handlers_frame_handler_on_resize")
public func onResize(_ w: UInt32, _ h: UInt32) { width = Float(w); height = Float(h) }

@_cdecl("exports_wasi_input_handlers_frame_handler_on_frame")
public func onFrame(_ nanos: UInt64) {
    if sharedGame == nil { sharedGame = GameLogic(size: BoardSize.fourByFour.rawValue) }
    let dt = lastFrameNanos == 0 ? 0.0 : Double(nanos &- lastFrameNanos) / 1_000_000_000.0
    lastFrameNanos = nanos

    let ctxOwn = wasi_canvas_embedding_get_context()
    let ctx = wasi_canvas_embedding_borrow_canvas_context(ctxOwn)
    let bufOwn = wasi_canvas_embedding_method_canvas_context_get_current_buffer(ctx)
    let canvas = wasi_canvas_draw_borrow_canvas(bufOwn)
    let gfxOwn = wasi_canvas_embedding_method_canvas_context_graphics(ctx)
    let gfx = wasi_canvas_draw_borrow_graphics_t(__handle: gfxOwn.__handle)
    let cg = CGContext(canvas: canvas, graphics: gfx)
    cg.clear(CGColor(red: 0.063, green: 0.078, blue: 0.094, alpha: 1))
    sink.cg = cg

    if !built, width > 0, height > 0 {
        renderWandrAppOnce(WandrHostApp(),
            options: .init(surface: CGSize(width: CGFloat(width), height: CGFloat(height)), sink: sink))
        built = true
        wlog("built AppGraph — real CompositeView")
    } else if animPending {
        animPending = wandrRenderFrame(dt); wandrRedraw()
    } else {
        wandrRedraw()
    }
    sink.cg = nil

    wasi_canvas_draw_canvas_drop_own(bufOwn)
    wasi_canvas_embedding_method_canvas_context_present(ctx)
    wasi_canvas_draw_graphics_drop_own(wasi_canvas_draw_own_graphics_t(__handle: gfxOwn.__handle))
    wasi_canvas_embedding_canvas_context_drop_own(ctxOwn)
}

@_cdecl("exports_wandr_ui_shell_frame_pacing_next_frame_delay")
public func nextFrameDelay() -> UInt32 { animPending ? 33 : 150 }

@_cdecl("exports_wasi_input_handlers_pointer_handler_on_pointer")
public func onPointer(_ ev: UnsafeMutablePointer<exports_wasi_input_handlers_pointer_handler_pointer_event_t>?) {
    guard let e = ev?.pointee else { return }
    switch e.kind {
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_DOWN):
        pointerPressing = true
        wandrSendPointer(phase: 0, x: Double(e.x), y: Double(e.y), serial: ptrSerial)
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_MOVE):
        if pointerPressing { wandrSendPointer(phase: 1, x: Double(e.x), y: Double(e.y), serial: ptrSerial) }
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_UP):
        pointerPressing = false
        wandrSendPointer(phase: 2, x: Double(e.x), y: Double(e.y), serial: ptrSerial)
        ptrSerial &+= 1
    default: break
    }
}
