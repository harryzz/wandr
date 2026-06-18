// Task 114 P2.3 — Swift renders on wandr-host THROUGH CoreGraphics. on_frame
// acquires the frame buffer (embedding handoff), wraps it in a `CGContext`
// (OpenCoreGraphics's CGContext implemented over wasi:canvas, see CoreGraphicsWasi),
// and draws with the CoreGraphics API — no raw wasi:canvas calls in the guest.
import CSwiftSpike
import OpenCoreGraphics   // vendored; CGContext implemented over wasi:canvas (re-exports Foundation's CGPoint/CGRect)

// The canvas-context is stable across frames — acquire once, re-borrow per frame
// (the keyguard pattern). wasm is single-threaded, so a global is fine.
// wasm32-wasip1 is single-threaded, so this global mutable state is safe to touch
// from the nonisolated @_cdecl exports (Swift 6 strict-concurrency opt-out).
nonisolated(unsafe) private var width: Float = 0
nonisolated(unsafe) private var height: Float = 0

@_cdecl("exports_wasi_input_handlers_frame_handler_on_resize")
public func onResize(_ w: UInt32, _ h: UInt32) {
    width = Float(w)
    height = Float(h)
}

@_cdecl("exports_wasi_input_handlers_frame_handler_on_frame")
public func onFrame(_ nanos: UInt64) {
    // Acquire the context fresh each frame (no cached own handle).
    let ctxOwn = wasi_canvas_embedding_get_context()
    let ctx = wasi_canvas_embedding_borrow_canvas_context(ctxOwn)
    let bufOwn = wasi_canvas_embedding_method_canvas_context_get_current_buffer(ctx)
    let canvas = wasi_canvas_draw_borrow_canvas(bufOwn)

    // From here on it's pure CoreGraphics — CGContext lowers to wasi:canvas.
    let cg = CGContext(canvas: canvas)
    let w = CGFloat(width), h = CGFloat(height)
    cg.clear(CGColor(red: 0.063, green: 0.078, blue: 0.094, alpha: 1)) // dark bg

    let m = max(16, w * 0.06)
    // Filled blue rect, sized from the surface (no hardcoded layout).
    cg.setFillColor(CGColor(red: 0.2, green: 0.4, blue: 0.8, alpha: 1))
    cg.fill(CGRect(x: m, y: m, width: max(0, w - 2 * m), height: h * 0.28))

    // Green stroked triangle, built with the CoreGraphics path API.
    cg.setStrokeColor(CGColor(red: 0.2, green: 0.667, blue: 0.333, alpha: 1))
    cg.setLineWidth(max(2, w * 0.015))
    cg.beginPath()
    cg.move(to: CGPoint(x: m, y: h * 0.74))
    cg.addLine(to: CGPoint(x: w - m, y: h * 0.74))
    cg.addLine(to: CGPoint(x: w * 0.5, y: h * 0.42))
    cg.closePath()
    cg.strokePath()

    wasi_canvas_draw_canvas_drop_own(bufOwn)
    wasi_canvas_embedding_method_canvas_context_present(ctx)
    wasi_canvas_embedding_canvas_context_drop_own(ctxOwn)
}

// Static scene → repaint rarely (host clamps; on-demand keeps idle CPU low).
@_cdecl("exports_wandr_ui_shell_frame_pacing_next_frame_delay")
public func nextFrameDelay() -> UInt32 { 1000 }

// Pointer events unused in this spike (scalars only — nothing to free).
@_cdecl("exports_wasi_input_handlers_pointer_handler_on_pointer")
public func onPointer(
    _ ev: UnsafeMutablePointer<exports_wasi_input_handlers_pointer_handler_pointer_event_t>?
) {}
