// Task 114 P2 — Swift drives REAL wasi:canvas. Exported `render` (host-called):
// get the embedding context, acquire the frame buffer, draw, present. Proves
// Swift C-interop handles wasi:canvas's rich ABI (paint records, rect, resource
// borrows). P2.2 builds OpenCoreGraphics's CGContext on top of exactly these calls.
import CSwiftSpike

@_cdecl("exports_swift_spike_render")
public func swiftSpikeRender() {
    // Embedding handoff: context -> current buffer (the canvas) -> draw -> present.
    let ctxOwn = wasi_canvas_embedding_get_context()
    let ctx = wasi_canvas_embedding_borrow_canvas_context(ctxOwn)
    let bufOwn = wasi_canvas_embedding_method_canvas_context_get_current_buffer(ctx)
    let canvas = wasi_canvas_draw_borrow_canvas(bufOwn)

    // clear to a dark background (ARGB 0xAARRGGBB)
    wasi_canvas_draw_method_canvas_clear(canvas, 0xFF10_1418)

    // a filled blue rect
    var fill = wasi_canvas_types_paint_t()           // zero-init: options = none
    fill.style = UInt8(WASI_CANVAS_TYPES_PAINT_STYLE_FILL)
    fill.color = 0xFF33_66CC
    fill.alpha = 255
    fill.anti_alias = true
    var r = wasi_canvas_types_rect_t(x: 20, y: 20, width: 160, height: 80)
    wasi_canvas_draw_method_canvas_draw_rect(canvas, &r, &fill)

    // a green stroked triangle, via an SVG path-data string
    var stroke = wasi_canvas_types_paint_t()
    stroke.style = UInt8(WASI_CANVAS_TYPES_PAINT_STYLE_STROKE)
    stroke.color = 0xFF33_AA55
    stroke.alpha = 255
    stroke.stroke_width = 4
    stroke.anti_alias = true
    var path = swift_spike_string_t()
    "M 40 140 L 160 140 L 100 200 Z".withCString { swift_spike_string_set(&path, $0) }
    wasi_canvas_draw_method_canvas_draw_path(
        canvas, &path, UInt8(WASI_CANVAS_TYPES_FILL_RULE_NONZERO), &stroke)

    // done: drop the frame buffer, present, drop the context.
    wasi_canvas_draw_canvas_drop_own(bufOwn)
    wasi_canvas_embedding_method_canvas_context_present(ctx)
    wasi_canvas_embedding_canvas_context_drop_own(ctxOwn)
}
