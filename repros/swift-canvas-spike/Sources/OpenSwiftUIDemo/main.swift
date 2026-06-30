// Phase 4b — OpenSwiftUI renders on wandr THROUGH a WandrDrawSink → CGContext → wasi:canvas.
// The OpenSwiftUI engine (AttributeGraph/Compute on wasm) lays out a real SwiftUI view, emits
// a DisplayList, and the new `.wandr` renderer walks it into our CGSink, which draws with the
// CoreGraphics API (OpenCoreGraphics's CGContext over wasi:canvas). Same frame plumbing as the
// hand-built spike (spike.swift) — only the draw source changed (SwiftUI instead of hand-coded).
import CSwiftSpike
import WandrCG
import OpenSwiftUI
@_spi(WandrRenderer) import OpenSwiftUI
#if canImport(WASILibc)
import WASILibc
#endif
@inline(never) func wlog(_ s: String) { fputs("WANDR-DEMO: \(s)\n", stderr); fflush(stderr) }

// MARK: - The SwiftUI app

// Playable: eleev/swiftui-2048's ACTUAL TileBoardView + GameLogic, swipe-driven (on_pointer).
// Reactivity is via @State (proven to work on wasm), NOT @ObservedObject — the ObservableObject
// subscription path corrupts the publisher ref on this wasm toolchain (OpenCombine Set.insert /
// PassthroughSubject.receive wild-pointer faults). The reactor owns the game; after a move it
// pushes the new board into the captured @State binding, which re-renders the view.
// Constructed explicitly in onFrame (plain reactor context), NOT a lazy global — the lazy
// global initializer (swift_once → GameLogic.init) runs in the OpenSwiftUI graph context and
// leaves tileMatrix nil on wasm. Mirrors the probe constructing it in Main.main.
nonisolated(unsafe) var sharedGame: GameLogic!
nonisolated(unsafe) var tickBinding: Binding<Int>?

struct ContentView: View {
    let game: GameLogic                 // plain ref (no @ObservedObject → no OpenCombine path)
    @State private var tick = 0         // re-render trigger; on_pointer bumps it after a move
    var body: some View {
        tickBinding = $tick             // expose so the reactor can request a re-render
        _ = tick                        // depend on tick so a bump re-runs body (the memcmp compare
                                        // fix makes the child TileBoardView re-eval on the new matrix
                                        // WITHOUT .id, so tiles are REUSED not re-created). NB: the old
                                        // note here blamed ".id → modalSpring transition → spring pow
                                        // (broken on aarch64-AOT)" — FALSIFIED 2026-06-30: spring
                                        // animation runs on aarch64 post-#14/#383 (see TileBoardView).
        return TileBoardView(
            matrix: game.tiles,         // read in BODY (game fully constructed), not in init
            tileEdge: game.lastGestureDirection.invertedEdge,
            tileBoardSize: game.boardSize
        )
    }
}

struct DemoApp: App {
    var body: some Scene { WindowGroup { ContentView(game: sharedGame) } }
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
nonisolated(unsafe) private var needsRender = false  // set after a move → next frame re-evaluates
nonisolated(unsafe) private var diagFrame = 0  // DIAGNOSTIC auto-move (device test w/o physical input)
nonisolated(unsafe) private var lastShapeCount = 0  // [wandr verify] draw-count delta baseline
nonisolated(unsafe) private var lastTextCount = 0

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
    // Construct the game HERE, in the plain reactor context, before the graph is built — so
    // GameLogic.init (→ reset → tileMatrix) runs outside the OpenSwiftUI graph eval.
    if sharedGame == nil { sharedGame = GameLogic(size: 4) }

    // [x86 repro] Drive the animation CONTINUOUSLY: a move every 3 frames, cycling all four
    // directions, reset every 64 moves to keep the board live → thousands of interpolatingSpring
    // animations, each firing the reentrant asyncAfter (WasmDispatchShim) on AnimationListener.
    // If that reentrancy is the crash, it must reproduce here on x86 (the shim is arch-independent).
    diagFrame &+= 1
    if diagFrame % 3 == 0, let g = sharedGame {
        let n = diagFrame / 3
        if n <= 5000 {  // bounded auto-play: 5000 moves then stop (desktop verification before device)
            let dirs: [Direction] = [.up, .left, .down, .right]
            wandrApplyChange {
                _ = g.move(dirs[n % 4])
                if n % 64 == 0 { g.reset() }
                tickBinding?.wrappedValue &+= 1
            }
            needsRender = true
            if n % 500 == 0 { wlog("AUTOPLAY \(n)/5000  score=\(g.score)") }
            if n == 5000 { wlog("AUTOPLAY: SURVIVED 5000 moves (no crash)") }
        }
    }

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
    } else if needsRender {
        // State changed (a move): re-RUN the graph (body re-reads the board, display list updates),
        // then redraw — wandrRender's own draw is skipped by the renderer's seed guard (the
        // display-list version doesn't bump for a @State change), but redraw() paints unconditionally.
        needsRender = false
        wandrRender()
        wandrRedraw()
    } else {
        // No change: cheap re-walk of the existing display list (double-buffering).
        wandrRedraw()
    }
    sink.cg = nil

    // [wandr verify] Prove tiles actually render: log shapes/texts drawn THIS frame. A gray empty
    // board would show ~0 shapes; a live 2048 board shows the grid + 16 cells + tile labels.
    if diagFrame % 200 == 0 {
        wlog("DRAWCOUNT frame=\(diagFrame) shapes=\(wandrDrawShapeCount - lastShapeCount) texts=\(wandrDrawTextCount - lastTextCount)")
    }
    lastShapeCount = wandrDrawShapeCount
    lastTextCount = wandrDrawTextCount

    wasi_canvas_draw_canvas_drop_own(bufOwn)
    wasi_canvas_embedding_method_canvas_context_present(ctx)
    wasi_canvas_draw_graphics_drop_own(wasi_canvas_draw_own_graphics_t(__handle: gfxOwn.__handle))
    wasi_canvas_embedding_canvas_context_drop_own(ctxOwn)
}

// Turn-based: poll gently (a swipe updates within ~150ms). Continuous high-fps GL on WSLg
// drops the display connection ("Connection reset by peer"); the static board was stable at 1fps.
@_cdecl("exports_wandr_ui_shell_frame_pacing_next_frame_delay")
public func nextFrameDelay() -> UInt32 { 150 }

// Swipe → GameLogic.move (down=0 start, up=1 release; dominant axis/sign picks direction).
nonisolated(unsafe) private var swipeStartX: Float = 0
nonisolated(unsafe) private var swipeStartY: Float = 0

@_cdecl("exports_wasi_input_handlers_pointer_handler_on_pointer")
public func onPointer(
    _ ev: UnsafeMutablePointer<exports_wasi_input_handlers_pointer_handler_pointer_event_t>?
) {
    guard let e = ev?.pointee else { return }
    switch e.kind {
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_DOWN):
        swipeStartX = e.x; swipeStartY = e.y
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_UP):
        let dx = e.x - swipeStartX, dy = e.y - swipeStartY
        let t: Float = 24
        let dir: Direction?
        if abs(dx) > abs(dy) {
            dir = dx > t ? .right : (dx < -t ? .left : nil)
        } else {
            dir = dy > t ? .down : (dy < -t ? .up : nil)
        }
        if let dir, let game = sharedGame {
            // Mutate inside an Update transaction so the @State write invalidates the graph,
            // then ask the next frame to re-RUN the graph (wandrRender) so the board repaints.
            wandrApplyChange {
                _ = game.move(dir)
                tickBinding?.wrappedValue &+= 1
            }
            needsRender = true
        }
    default: break
    }
}
