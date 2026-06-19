// Phase 4b — OpenSwiftUI renders on wandr THROUGH a WandrDrawSink → CGContext → wasi:canvas.
// The OpenSwiftUI engine (AttributeGraph/Compute on wasm) lays out a real SwiftUI view, emits
// a DisplayList, and the new `.wandr` renderer walks it into our CGSink, which draws with the
// CoreGraphics API (OpenCoreGraphics's CGContext over wasi:canvas). Same frame plumbing as the
// hand-built spike (spike.swift) — only the draw source changed (SwiftUI instead of hand-coded).
import CSwiftSpike
import WandrCG
import OpenSwiftUI
@_spi(WandrRenderer) import OpenSwiftUI

// MARK: - The SwiftUI app

// 2048 tile palette (the game's canonical look — one named source of truth).
func tileColors(_ v: Int) -> (bg: Color, fg: Color) {
    let dark = Color(red: 0.47, green: 0.43, blue: 0.40)
    let light = Color(red: 0.98, green: 0.96, blue: 0.93)
    switch v {
    case 2:    return (Color(red: 0.93, green: 0.89, blue: 0.85), dark)
    case 4:    return (Color(red: 0.93, green: 0.88, blue: 0.78), dark)
    case 8:    return (Color(red: 0.95, green: 0.69, blue: 0.47), light)
    case 16:   return (Color(red: 0.96, green: 0.58, blue: 0.39), light)
    case 32:   return (Color(red: 0.96, green: 0.49, blue: 0.37), light)
    case 64:   return (Color(red: 0.96, green: 0.37, blue: 0.23), light)
    case 128:  return (Color(red: 0.93, green: 0.81, blue: 0.45), light)
    case 256:  return (Color(red: 0.93, green: 0.80, blue: 0.38), light)
    case 512:  return (Color(red: 0.93, green: 0.78, blue: 0.31), light)
    case 1024: return (Color(red: 0.93, green: 0.77, blue: 0.25), light)
    case 2048: return (Color(red: 0.93, green: 0.76, blue: 0.18), light)
    default:   return (Color(red: 0.80, green: 0.76, blue: 0.71), dark) // empty
    }
}

// Idiomatic: ForEach + ViewBuilder conditionals now work (fixed indirect_pointer_vector
// uninitialized _data, Subgraph.index shadow stub, and the .effect-frame offset in the walker).
struct TileCell: View {
    let value: Int
    var body: some View {
        let colors = tileColors(value)
        return ZStack {
            RoundedRectangle(cornerRadius: 6).fill(colors.bg)
            if value > 0 {
                Text("\(value)")
                    .font(.system(size: 30, weight: .bold))
                    .foregroundColor(colors.fg)
            }
        }
        .frame(width: 76, height: 76)
    }
}

struct ContentView: View {
    let grid = [[2, 4, 8, 16], [32, 64, 128, 256], [512, 1024, 2048, 0], [0, 0, 2, 4]]
    var body: some View {
        VStack(spacing: 8) {
            Text("2048").font(.system(size: 44, weight: .bold)).foregroundColor(.white)
            ForEach(0..<4, id: \.self) { r in
                HStack(spacing: 8) {
                    ForEach(0..<4, id: \.self) { c in
                        TileCell(value: grid[r][c])
                    }
                }
            }
        }
    }
}

struct DemoApp: App {
    var body: some Scene { WindowGroup { ContentView() } }
}

// MARK: - WandrDrawSink over a CGContext

// The sink receives fully-resolved primitives from OpenSwiftUI's DisplayList walk and draws
// them with CoreGraphics. `cg` is repointed at the current back-buffer each frame.
final class CGSink: WandrDrawSink {
    nonisolated(unsafe) var cg: CGContext?

    func beginFrame(width: Double, height: Double, version: UInt32) {}

    func fillRect(
        x: Double, y: Double, width: Double, height: Double,
        red: Float, green: Float, blue: Float, opacity: Float
    ) {
        guard let cg else { return }
        cg.setFillColor(CGColor(
            red: CGFloat(red), green: CGFloat(green), blue: CGFloat(blue), alpha: CGFloat(opacity)
        ))
        cg.fill(CGRect(x: CGFloat(x), y: CGFloat(y), width: CGFloat(width), height: CGFloat(height)))
    }

    func drawText(
        _ text: String, x: Double, y: Double, width: Double, height: Double,
        fontSize: Double, red: Float, green: Float, blue: Float, opacity: Float
    ) {
        guard let cg else { return }
        // CGContext.drawString lowers to wasi:canvas text/paragraph (Skia shapes + draws).
        // Draw at the given size (which matches the reserved band height), so the next VStack
        // child fills BELOW the text rather than over it.
        cg.drawString(
            text,
            at: CGPoint(x: CGFloat(x), y: CGFloat(y)),
            size: CGFloat(fontSize),
            color: CGColor(red: CGFloat(red), green: CGFloat(green), blue: CGFloat(blue), alpha: CGFloat(opacity)),
            maxWidth: CGFloat(width)
        )
    }

    func endFrame() {}
}

// MARK: - Reactor state (wasm32-wasip1 single-threaded ⇒ globals are safe)

nonisolated(unsafe) private let sink = CGSink()
nonisolated(unsafe) private var width: Float = 0
nonisolated(unsafe) private var height: Float = 0
nonisolated(unsafe) private var built = false

@_cdecl("exports_wasi_input_handlers_frame_handler_on_resize")
public func onResize(_ w: UInt32, _ h: UInt32) {
    width = Float(w)
    height = Float(h)
    // NOTE: do NOT rebuild here. renderWandrAppOnce builds the AppGraph and sets the
    // once-only `AppGraph.shared`; calling it twice fatalErrors ("may only be set once").
    // The graph is built exactly once (first frame with valid dims); resizes just re-render
    // at the original layout. (Proper resize-relayout = a setSize on WandrRendererHost — TODO.)
}

@_cdecl("exports_wasi_input_handlers_frame_handler_on_frame")
public func onFrame(_ nanos: UInt64) {
    // Acquire the context + buffer fresh each frame (the keyguard pattern).
    let ctxOwn = wasi_canvas_embedding_get_context()
    let ctx = wasi_canvas_embedding_borrow_canvas_context(ctxOwn)
    let bufOwn = wasi_canvas_embedding_method_canvas_context_get_current_buffer(ctx)
    let canvas = wasi_canvas_draw_borrow_canvas(bufOwn)
    let gfxOwn = wasi_canvas_embedding_method_canvas_context_graphics(ctx)
    let gfx = wasi_canvas_draw_borrow_graphics_t(__handle: gfxOwn.__handle)

    let cg = CGContext(canvas: canvas, graphics: gfx)
    cg.clear(CGColor(red: 0.063, green: 0.078, blue: 0.094, alpha: 1)) // dark bg
    sink.cg = cg

    if !built, width > 0, height > 0 {
        // Build the OpenSwiftUI graph + render once. The host is retained for the
        // process lifetime; subsequent frames just re-walk the display list.
        renderWandrAppOnce(
            DemoApp(),
            options: .init(surface: CGSize(width: CGFloat(width), height: CGFloat(height)), sink: sink)
        )
        built = true
    } else {
        // Repaint the (static) scene into the new back-buffer under double-buffering.
        wandrRedraw()
    }
    sink.cg = nil

    wasi_canvas_draw_canvas_drop_own(bufOwn)
    wasi_canvas_embedding_method_canvas_context_present(ctx)
    wasi_canvas_draw_graphics_drop_own(wasi_canvas_draw_own_graphics_t(__handle: gfxOwn.__handle))
    wasi_canvas_embedding_canvas_context_drop_own(ctxOwn)
}

// Static scene → repaint rarely (host clamps; on-demand keeps idle CPU low).
@_cdecl("exports_wandr_ui_shell_frame_pacing_next_frame_delay")
public func nextFrameDelay() -> UInt32 { 1000 }

// Pointer events unused for the first pixel (scalars only — nothing to free).
@_cdecl("exports_wasi_input_handlers_pointer_handler_on_pointer")
public func onPointer(
    _ ev: UnsafeMutablePointer<exports_wasi_input_handlers_pointer_handler_pointer_event_t>?
) {}
