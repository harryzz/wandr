// Task 114 P2.3 — Swift renders on wandr-host THROUGH CoreGraphics. on_frame
// acquires the frame buffer (embedding handoff), wraps it in a `CGContext`
// (OpenCoreGraphics's CGContext implemented over wasi:canvas, see CoreGraphicsWasi),
// and draws with the CoreGraphics API — no raw wasi:canvas calls in the guest.
import CSwiftSpike
import CWASICanvas
import OpenCoreGraphicsWASICanvas

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

    // Option-B prototype: build an OpenSwiftUI-style DisplayList and render it through the
    // wandr backend (render(_:into:)). Exercises content (color/shape/text) + the recursive
    // effect tree (opacity/clip/transform, incl. nesting) → CGContext → wasi:canvas.
    let blue  = CGColor(red: 0.20, green: 0.40, blue: 0.85, alpha: 1)
    let green = CGColor(red: 0.20, green: 0.75, blue: 0.40, alpha: 1)
    let amber = CGColor(red: 0.95, green: 0.65, blue: 0.20, alpha: 1)
    let white = CGColor(red: 0.92, green: 0.94, blue: 1.00, alpha: 1)
    let red   = CGColor(red: 0.90, green: 0.25, blue: 0.30, alpha: 1)

    func rc(_ x: CGFloat, _ y: CGFloat, _ rw: CGFloat, _ rh: CGFloat) -> CGRect {
        CGRect(x: x, y: y, width: rw, height: rh)
    }
    func tri(_ rw: CGFloat, _ rh: CGFloat) -> CGPath {   // triangle in a local rw×rh box
        CGPath(elements: [.moveToPoint(CGPoint(x: rw / 2, y: 0)),
                          .addLineToPoint(CGPoint(x: rw, y: rh)),
                          .addLineToPoint(CGPoint(x: 0, y: rh)),
                          .closeSubpath])
    }
    let cw = w - 2 * m

    let list = DisplayList([
        // .content(.color) — frame-filling solid
        .init(.content(.color(blue)), frame: rc(m, h * 0.04, cw, h * 0.10)),
        // .content(.shape) — filled triangle
        .init(.content(.shape(tri(cw, h * 0.12), green)), frame: rc(m, h * 0.17, cw, h * 0.12)),
        // .content(.text)
        .init(.content(.text("DisplayList → CGContext → wasi:canvas", max(14, w * 0.045), white)),
              frame: rc(m, h * 0.31, cw, h * 0.06)),
        // .effect(.opacity) wrapping a sub-list (amber at 45%)
        .init(.effect(.opacity(115), DisplayList([
            .init(.content(.color(amber)), frame: rc(0, 0, cw, h * 0.10)),
        ])), frame: rc(m, h * 0.39, cw, h * 0.10)),
        // .effect(.clip) — red sub-list clipped to a triangle
        .init(.effect(.clip(tri(cw, h * 0.12)), DisplayList([
            .init(.content(.color(red)), frame: rc(0, 0, cw, h * 0.12)),
        ])), frame: rc(m, h * 0.51, cw, h * 0.12)),
        // .effect(.transform) — rotated sub-list, with a NESTED .opacity (recursion depth)
        .init(.effect(.transform(CGAffineTransform(rotationAngle: 0.10)), DisplayList([
            .init(.content(.color(green)), frame: rc(0, 0, cw * 0.5, h * 0.09)),
            .init(.effect(.opacity(150), DisplayList([
                .init(.content(.color(white)), frame: rc(0, 0, cw * 0.25, h * 0.05)),
            ])), frame: rc(cw * 0.55, 0, cw * 0.25, h * 0.05)),
        ])), frame: rc(m, h * 0.66, cw, h * 0.12)),
        // bottom label
        .init(.content(.text("wandr DisplayList backend (Option B)", max(12, w * 0.04), white)),
              frame: rc(m, h * 0.90, cw, h * 0.06)),
    ])
    render(list, into: cg)

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
