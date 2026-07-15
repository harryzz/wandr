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

// [wandr] Everything below TOUCHES wasi:canvas (the CGSink over CGContext + the reactor @_cdecl
// frame/pointer exports). The -DWANDR_HEADLESS deterministic driver has no canvas, so gate it out —
// otherwise the module still imports wasi:canvas and wasmtime refuses to instantiate the plain command.
#if !WANDR_HEADLESS
// MARK: - WandrDrawSink over a CGContext (OpenSwiftUI DisplayList → CoreGraphics → wasi:canvas)
final class CGSink: WandrDrawSink {
    nonisolated(unsafe) var cg: CGContext?
    // Decoded-image cache, keyed by the DisplayList's per-CGImage identity (see
    // WandrDisplayListRenderer's `.image` case) — a static bundle image redraws every frame; decode
    // host-side (wasi:canvas graphics.decode-image) once, reuse the resource handle thereafter.
    nonisolated(unsafe) private var imageCache: [String: CGImage] = [:]
    func beginFrame(width: Double, height: Double, version: UInt32) {}
    func fillRect(x: Double, y: Double, width: Double, height: Double,
                  red: Float, green: Float, blue: Float, opacity: Float) {
        guard let cg else { return }
        cg.setFillColor(CGColor(red: CGFloat(red), green: CGFloat(green), blue: CGFloat(blue), alpha: CGFloat(opacity)))
        cg.fill(CGRect(x: CGFloat(x), y: CGFloat(y), width: CGFloat(width), height: CGFloat(height)))
    }
    func drawText(_ text: String, x: Double, y: Double, width: Double, height: Double,
                  fontSize: Double, red: Float, green: Float, blue: Float, opacity: Float,
                  fontFamily: String) {
        guard let cg else { return }
        cg.drawString(text, at: CGPoint(x: CGFloat(x), y: CGFloat(y)), size: CGFloat(fontSize),
                      color: CGColor(red: CGFloat(red), green: CGFloat(green), blue: CGFloat(blue), alpha: CGFloat(opacity)),
                      maxWidth: CGFloat(width), family: fontFamily)
    }
    func fillPath(svgPath: String, x: Double, y: Double, width: Double, height: Double,
                  red: Float, green: Float, blue: Float, opacity: Float) {
        guard let cg else { return }
        cg.setFillColor(CGColor(red: CGFloat(red), green: CGFloat(green), blue: CGFloat(blue), alpha: CGFloat(opacity)))
        cg.fill(svgPath: svgPath)
    }
    func pushClip(svgPath: String) {
        guard let cg else { return }
        cg.saveGState()
        cg.clip(svgPath: svgPath)
    }
    func popClip() { cg?.restoreGState() }
    var wandrSupportsProjection: Bool { true }
    func saveState() { cg?.saveGState() }
    func restoreState() { cg?.restoreGState() }
    func concat(m00: Double, m01: Double, m02: Double,
                m10: Double, m11: Double, m12: Double,
                m20: Double, m21: Double, m22: Double) {
        cg?.concat3x3(CGFloat(m00), CGFloat(m01), CGFloat(m02),
                      CGFloat(m10), CGFloat(m11), CGFloat(m12),
                      CGFloat(m20), CGFloat(m21), CGFloat(m22))
    }
    func fillPathShadow(svgPath: String, dx: Double, dy: Double, blur: Double,
                        red: Float, green: Float, blue: Float, opacity: Float) {
        cg?.fillShadowPath(svgPath, dx: CGFloat(dx), dy: CGFloat(dy), blur: CGFloat(blur),
                           color: CGColor(red: CGFloat(red), green: CGFloat(green), blue: CGFloat(blue), alpha: CGFloat(opacity)))
    }
    func drawImage(data: [UInt8], name: String, pixelWidth: Int, pixelHeight: Int,
                   x: Double, y: Double, width: Double, height: Double, opacity: Float) {
        guard let cg else { return }
        let image: CGImage?
        if let cached = imageCache[name] {
            image = cached
        } else {
            image = cg.decodeImage(encodedData: data, pixelWidth: pixelWidth, pixelHeight: pixelHeight)
            if image == nil {
                wlog("drawImage: decodeImage FAILED for '\(name)' (\(data.count) bytes, \(pixelWidth)x\(pixelHeight))")
            }
            imageCache[name] = image
        }
        guard let image else { return }
        cg.drawImageFitting(image, in: CGRect(x: CGFloat(x), y: CGFloat(y), width: CGFloat(width), height: CGFloat(height)),
                            opacity: opacity)
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
    // A gesture action may have mutated state and/or scheduled a `withAnimation` transition
    // (menu slide, tile move/merge). Kick the animation loop so onFrame re-runs the graph and
    // ADVANCES the animation clock via wandrRenderFrame — otherwise the steady-state redraw-only
    // path leaves animated properties frozen at their old values (the change never appears).
    animPending = true
}
#endif // !WANDR_HEADLESS
