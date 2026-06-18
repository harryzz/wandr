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
    // This scene exercises the 3 emulated gaps: dash, offset+color shadow, mask-clip.
    let cg = CGContext(canvas: canvas)
    let w = CGFloat(width), h = CGFloat(height)
    let m = max(16, w * 0.06)
    cg.clear(CGColor(red: 0.063, green: 0.078, blue: 0.094, alpha: 1)) // dark bg

    // (1) DASH — a dashed rounded line border via the CoreGraphics dash API.
    cg.setStrokeColor(CGColor(red: 0.85, green: 0.85, blue: 0.9, alpha: 1))
    cg.setLineWidth(max(3, w * 0.012))
    cg.setLineDash(phase: 0, lengths: [w * 0.05, w * 0.03])
    cg.beginPath()
    cg.addRect(CGRect(x: m, y: h * 0.05, width: w - 2 * m, height: h * 0.14))
    cg.strokePath()
    cg.setLineDash(phase: 0, lengths: [])   // solid again

    // (2) SHADOW — a blue rect casting an offset, blurred, COLORED (cyan) shadow.
    cg.setShadow(offset: (w * 0.03, h * 0.012), blur: w * 0.02,
                 color: CGColor(red: 0.2, green: 0.9, blue: 1.0, alpha: 0.8))
    cg.setFillColor(CGColor(red: 0.2, green: 0.4, blue: 0.8, alpha: 1))
    cg.fill(CGRect(x: w * 0.25, y: h * 0.28, width: w * 0.5, height: h * 0.14))
    cg.clearShadow()

    // (3) MASK-CLIP — clip a green rect to an oval ALPHA mask. Idiom: in a
    // transparency layer, draw the mask (the oval) first, then draw the content
    // with src-in so it survives only where the mask's alpha is (and at its alpha).
    // The oval mask is 60% alpha → a soft, half-strength green oval (proves it's an
    // alpha mask, not a binary clip).
    cg.beginTransparencyLayer()
    cg.setBlendMode(.normal)
    cg.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 0.6)) // alpha mask
    cg.fillEllipse(in: CGRect(x: w * 0.2, y: h * 0.55, width: w * 0.6, height: h * 0.24))
    cg.setBlendMode(.sourceIn)
    cg.setFillColor(CGColor(red: 0.2, green: 0.7, blue: 0.35, alpha: 1)) // content
    cg.fill(CGRect(x: m, y: h * 0.52, width: w - 2 * m, height: h * 0.3))
    cg.setBlendMode(.normal)
    cg.endTransparencyLayer()

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
