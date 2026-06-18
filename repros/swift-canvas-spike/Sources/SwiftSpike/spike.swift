// Task 114 P2.2 — Swift renders on wandr-host. Implements the host-driven reactor
// exports (frame-handler + frame-pacing + a pointer stub) via @_cdecl; on-frame
// acquires the frame buffer through the embedding handoff, draws with wasi:canvas,
// and presents. Runs unchanged on wandr-host's skia wasi:canvas (desktop → device).
import CSwiftSpike

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

    wasi_canvas_draw_method_canvas_clear(canvas, 0xFF10_1418) // dark bg

    // Filled blue rounded-ish rect, sized from the surface so it's visible at any
    // resolution (no hardcoded layout).
    let m: Float = max(16, width * 0.06)
    var fill = wasi_canvas_types_paint_t()
    fill.style = UInt8(WASI_CANVAS_TYPES_PAINT_STYLE_FILL)
    fill.color = 0xFF33_66CC
    fill.alpha = 255
    fill.anti_alias = true
    var r = wasi_canvas_types_rect_t(x: m, y: m, width: max(0, width - 2 * m), height: height * 0.28)
    wasi_canvas_draw_method_canvas_draw_rect(canvas, &r, &fill)

    // Green stroked triangle below it, via an SVG path-data string.
    let cx = width * 0.5
    let top = height * 0.42
    let bot = height * 0.74
    var stroke = wasi_canvas_types_paint_t()
    stroke.style = UInt8(WASI_CANVAS_TYPES_PAINT_STYLE_STROKE)
    stroke.color = 0xFF33_AA55
    stroke.alpha = 255
    stroke.stroke_width = max(2, width * 0.015)
    stroke.anti_alias = true
    let d = "M \(m) \(bot) L \(width - m) \(bot) L \(cx) \(top) Z"
    var path = swift_spike_string_t()
    d.withCString { swift_spike_string_set(&path, $0) }
    wasi_canvas_draw_method_canvas_draw_path(
        canvas, &path, UInt8(WASI_CANVAS_TYPES_FILL_RULE_NONZERO), &stroke)

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
