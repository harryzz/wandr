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
    // Acquire the context + resource factory fresh each frame (no cached handles).
    let ctxOwn = wasi_canvas_embedding_get_context()
    let ctx = wasi_canvas_embedding_borrow_canvas_context(ctxOwn)
    let bufOwn = wasi_canvas_embedding_method_canvas_context_get_current_buffer(ctx)
    let canvas = wasi_canvas_draw_borrow_canvas(bufOwn)
    let gfxOwn = wasi_canvas_embedding_method_canvas_context_graphics(ctx)
    let gfx = wasi_canvas_draw_borrow_graphics_t(__handle: gfxOwn.__handle)

    // From here on it's pure CoreGraphics — CGContext lowers to wasi:canvas.
    let cg = CGContext(canvas: canvas, graphics: gfx)
    let w = CGFloat(width), h = CGFloat(height)
    let m = max(16, w * 0.06)
    cg.clear(CGColor(red: 0.063, green: 0.078, blue: 0.094, alpha: 1)) // dark bg

    // (1) DASH — dashed border via the CoreGraphics dash API.
    cg.setStrokeColor(CGColor(red: 0.85, green: 0.85, blue: 0.9, alpha: 1))
    cg.setLineWidth(max(3, w * 0.012))
    cg.setLineDash(phase: 0, lengths: [w * 0.05, w * 0.03])
    cg.beginPath()
    cg.addRect(CGRect(x: m, y: h * 0.04, width: w - 2 * m, height: h * 0.10))
    cg.strokePath()
    cg.setLineDash(phase: 0, lengths: [])

    // (1b) IMAGE — a generated RGBA8 bitmap (R ramps x, G ramps y) drawn inside
    // the dashed border via makeImage + draw(_:in:).
    let iw = 64, ih = 64
    var px = [UInt8]()
    px.reserveCapacity(iw * ih * 4)
    for y in 0..<ih {
        for x in 0..<iw {
            px.append(UInt8(x * 255 / (iw - 1)))   // R
            px.append(UInt8(y * 255 / (ih - 1)))   // G
            px.append(160)                          // B
            px.append(255)                          // A
        }
    }
    if let img = cg.makeImage(rgba: px, width: iw, height: ih) {
        cg.draw(img, in: CGRect(x: m * 1.5, y: h * 0.055, width: w - 3 * m, height: h * 0.075))
    }

    // (2) SHADOW — blue rect with an offset, blurred, cyan shadow.
    cg.setShadow(offset: (w * 0.03, h * 0.01), blur: w * 0.02,
                 color: CGColor(red: 0.2, green: 0.9, blue: 1.0, alpha: 0.8))
    cg.setFillColor(CGColor(red: 0.2, green: 0.4, blue: 0.8, alpha: 1))
    cg.fill(CGRect(x: w * 0.28, y: h * 0.18, width: w * 0.44, height: h * 0.09))
    cg.clearShadow()

    // (3) LINEAR GRADIENT — blue→cyan across a rect.
    let lin = CGGradient(colors: [CGColor(red: 0.1, green: 0.3, blue: 0.9, alpha: 1),
                                  CGColor(red: 0.1, green: 0.9, blue: 0.9, alpha: 1)])
    cg.drawLinearGradient(lin, start: CGPoint(x: m, y: 0), end: CGPoint(x: w - m, y: 0),
                          in: CGRect(x: m, y: h * 0.31, width: w - 2 * m, height: h * 0.10))

    // (4) CLIP + RADIAL GRADIENT — radial gradient clipped to a triangle.
    cg.saveGState()
    cg.beginPath()
    cg.move(to: CGPoint(x: m, y: h * 0.63))
    cg.addLine(to: CGPoint(x: w - m, y: h * 0.63))
    cg.addLine(to: CGPoint(x: w * 0.5, y: h * 0.45))
    cg.closePath()
    cg.clip()   // clip subsequent drawing to the triangle
    let rad = CGGradient(colors: [CGColor(red: 1, green: 1, blue: 0.9, alpha: 1),
                                  CGColor(red: 0.6, green: 0.2, blue: 0.8, alpha: 1)])
    cg.drawRadialGradient(rad, center: CGPoint(x: w * 0.5, y: h * 0.57), radius: w * 0.45,
                          in: CGRect(x: 0, y: h * 0.44, width: w, height: h * 0.20))
    cg.restoreGState()

    // (5) MASK-CLIP — green content clipped to a soft (60% alpha) oval mask.
    cg.beginTransparencyLayer()
    cg.setBlendMode(.normal)
    cg.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 0.6))
    cg.fillEllipse(in: CGRect(x: w * 0.22, y: h * 0.69, width: w * 0.56, height: h * 0.20))
    cg.setBlendMode(.sourceIn)
    cg.setFillColor(CGColor(red: 0.2, green: 0.7, blue: 0.35, alpha: 1))
    cg.fill(CGRect(x: m, y: h * 0.67, width: w - 2 * m, height: h * 0.24))
    cg.setBlendMode(.normal)
    cg.endTransparencyLayer()

    // (6) TEXT — host-shaped paragraph (wasi:canvas/layout) via CGContext.drawString.
    cg.drawString("Swift · OpenCoreGraphics → wasi:canvas",
                  at: CGPoint(x: m, y: h * 0.925), size: max(14, w * 0.05),
                  color: CGColor(red: 0.92, green: 0.94, blue: 1.0, alpha: 1),
                  maxWidth: w - 2 * m)

    wasi_canvas_draw_canvas_drop_own(bufOwn)
    wasi_canvas_embedding_method_canvas_context_present(ctx)
    wasi_canvas_draw_graphics_drop_own(wasi_canvas_draw_own_graphics_t(__handle: gfxOwn.__handle))
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
