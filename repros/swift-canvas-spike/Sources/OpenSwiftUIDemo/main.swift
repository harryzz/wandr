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

    private var titleColor: Color { Color(red: 0.47, green: 0.43, blue: 0.40, opacity: 1) }
    private var labelColor: Color { Color(red: 0.93, green: 0.89, blue: 0.85, opacity: 1) }
    private var boxColor:   Color { Color(red: 0.73, green: 0.68, blue: 0.63, opacity: 1) }
    private var hintColor:  Color { Color(red: 0.58, green: 0.56, blue: 0.53, opacity: 1) }

    // A score/best readout box (mirrors eleev's ScoreView header).
    private func scoreBox(_ label: String, _ value: Int, _ w: CGFloat) -> some View {
        VStack(spacing: 0) {
            Text(label).font(.system(size: w * 0.026, weight: .medium)).foregroundColor(labelColor)
            Text("\(value)").font(.system(size: w * 0.050, weight: .heavy)).foregroundColor(.white)
        }
        .frame(width: w * 0.25, height: w * 0.14)
        .background(Rectangle().fill(boxColor).cornerRadius(w * 0.012))
        // Real gesture: tap a score box → toggle autoplay (replaces the hand-rolled scoreRect hit-test).
        .onTapGesture {
            wandrApplyChange { autoplayOn.toggle(); tickBinding?.wrappedValue &+= 1 }
            animPending = true
        }
    }

    // Confirm-dialog button colors — distinct fills so the CGSink can recover their rects (the
    // literals are mirrored in fillRect). Greens/blue-grey, away from board/box/tile colors.
    private var yesColor: Color { Color(red: 0.30, green: 0.69, blue: 0.31, opacity: 1) }
    private var noColor:  Color { Color(red: 0.35, green: 0.38, blue: 0.45, opacity: 1) }
    private func confirmButton(_ label: String, _ color: Color, _ w: CGFloat) -> some View {
        Text(label).font(.system(size: w * 0.05, weight: .heavy)).foregroundColor(.white)
            .frame(width: w * 0.30, height: w * 0.15)
            .background(Rectangle().fill(color).cornerRadius(w * 0.02))
    }

    var body: some View {
        tickBinding = $tick             // expose so the reactor can request a re-render
        _ = tick                        // depend on tick so a bump re-runs body (the memcmp compare fix
                                        // makes the child TileBoardView re-eval on the new matrix WITHOUT
                                        // .id, so tiles are REUSED not re-created). The old "spring pow
                                        // broken on aarch64-AOT" note here was FALSIFIED 2026-06-30.
        return GeometryReader { proxy in
            let w = proxy.size.width
            ZStack {
            VStack(spacing: w * 0.03) {
                // Header: "2048" title + SCORE / BEST boxes (like the eleev gif).
                HStack(alignment: .center, spacing: w * 0.015) {
                    Text("2048").font(.system(size: w * 0.10, weight: .heavy)).foregroundColor(titleColor)
                        // Real gesture: tap the title → open the new-game confirm dialog.
                        .onTapGesture {
                            wandrApplyChange { confirmNewGame = true; tickBinding?.wrappedValue &+= 1 }
                            animPending = true
                        }
                    Spacer()
                    scoreBox("SCORE", game.score, w)
                    scoreBox("BEST", max(bestScore, game.score), w)
                }
                .frame(height: w * 0.16)
                // Mode hint + how to reach the autoplay/new-game controls (the top band, see onPointer).
                Text(autoplayOn ? "AUTO ON  -  tap 2048 = new game  -  tap a score box = stop"
                                : "swipe to play  -  tap a score box = autoplay")
                    .font(.system(size: w * 0.032, weight: .medium))
                    .foregroundColor(hintColor)
                ZStack {
                    TileBoardView(
                        matrix: game.tiles,     // read in BODY (game fully constructed), not in init
                        tileEdge: game.lastGestureDirection.invertedEdge,
                        tileBoardSize: game.boardSize
                    )
                    // Game over (board full, no merges left) → dim + prompt. The board's DragGesture
                    // below handles the restart on any swipe (no separate overlay gesture — two
                    // gestures on one area would need arbitration, which is deferred).
                    if !game.tiles.isMovePossible() {
                        Rectangle().fill(Color(red: 0.05, green: 0.06, blue: 0.07, opacity: 0.72))
                        VStack(spacing: w * 0.025) {
                            Text("GAME OVER")
                                .font(.system(size: w * 0.10, weight: .heavy)).foregroundColor(.white)
                            Text("swipe to play again")
                                .font(.system(size: w * 0.040, weight: .medium)).foregroundColor(labelColor)
                        }
                    }
                }
                // Real gesture: swipe the board → move (replaces the hand-rolled boardRect+delta).
                // minimumDistance gates out taps, so only a real swipe acts; onEnded picks direction.
                // At game over, any swipe restarts the game.
                .gesture(
                    DragGesture(minimumDistance: w * 0.05).onEnded { v in
                        guard !confirmNewGame else { return }
                        if !game.tiles.isMovePossible() {
                            wandrApplyChange { autoplayOn = false; game.reset(); tickBinding?.wrappedValue &+= 1 }
                            animPending = true
                            return
                        }
                        let t = v.translation
                        let dir: Direction = abs(t.width) > abs(t.height)
                            ? (t.width > 0 ? .right : .left)
                            : (t.height > 0 ? .down : .up)
                        wandrApplyChange { autoplayOn = false; _ = game.move(dir); tickBinding?.wrappedValue &+= 1 }
                        if game.score > bestScore { bestScore = game.score }
                        animPending = true
                    }
                )
            }
            // New-game confirmation (tapping "2048" opens this; nothing resets without a YES).
            if confirmNewGame {
                // Dim backdrop — a modal no-op tap so taps that miss the buttons don't fall through
                // to the board/header underneath.
                Rectangle().fill(Color(red: 0.05, green: 0.06, blue: 0.07, opacity: 0.80))
                    .onTapGesture { }
                VStack(spacing: w * 0.06) {
                    Text("New game?").font(.system(size: w * 0.085, weight: .heavy)).foregroundColor(.white)
                    HStack(spacing: w * 0.07) {
                        confirmButton("NO", noColor, w)
                            .onTapGesture {
                                wandrApplyChange { confirmNewGame = false; tickBinding?.wrappedValue &+= 1 }
                                animPending = true
                            }
                        confirmButton("YES", yesColor, w)
                            .onTapGesture {
                                wandrApplyChange { autoplayOn = false; game.reset(); confirmNewGame = false; tickBinding?.wrappedValue &+= 1 }
                                animPending = true
                            }
                    }
                }
            }
            }
        }
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
nonisolated(unsafe) private var animPending = false  // an animation is in flight → drive frames fast
nonisolated(unsafe) private var diagFrame = 0        // frame counter (DRAWCOUNT logging only)
nonisolated(unsafe) private var moveCount = 0        // auto-play move number (wall-clock paced)
nonisolated(unsafe) private var lastMoveNanos: UInt64 = 0   // wall-clock of the last auto-move
nonisolated(unsafe) private var lastFrameNanos: UInt64 = 0  // wall-clock of the last frame (→ dt)
nonisolated(unsafe) private var lastShapeCount = 0  // [wandr verify] draw-count delta baseline
nonisolated(unsafe) private var lastTextCount = 0
nonisolated(unsafe) private var autoplayOn = false  // demo autoplay — DEFAULT OFF (user plays by swiping)
nonisolated(unsafe) private var bestScore = 0       // peak score across the session (header "BEST" box)
nonisolated(unsafe) private var confirmNewGame = false  // show the "New game?" yes/no dialog
// [PHASE-A PROBE] does a tap routed through OpenSwiftUI's gesture pipeline fire a .onTapGesture?
nonisolated(unsafe) private var ptrSerial = 0

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

    diagFrame &+= 1
    // dt since the last frame → drives the animation clock so springs INTERPOLATE (vs snap).
    let dt = lastFrameNanos == 0 ? 0.0 : Double(nanos &- lastFrameNanos) / 1_000_000_000.0
    lastFrameNanos = nanos

    // Autoplay — DEFAULT OFF (toggled from the header, see onPointer). One move every 450ms,
    // wall-clock paced and DECOUPLED from the frame rate so the spring animates smoothly between
    // moves (the old frame-count cadence sped the game up whenever we sped the frames up).
    if built, autoplayOn, let g = sharedGame,
       lastMoveNanos == 0 || nanos &- lastMoveNanos >= 450_000_000 {
        // gated on `built` — the first move needs the graph's subgraph (AppGraph.shared) set up.
        lastMoveNanos = nanos
        if !g.tiles.isMovePossible() {
            // Board stuck (game over) → restart so autoplay keeps running.
            wandrApplyChange { g.reset(); tickBinding?.wrappedValue &+= 1 }
        } else {
            moveCount &+= 1
            let dirs: [Direction] = [.up, .left, .down, .right]
            var st: GameLogic.State = .none
            wandrApplyChange {
                st = g.move(dirs[moveCount % 4])
                tickBinding?.wrappedValue &+= 1
            }
            wlog("AUTO n=\(moveCount) dir=\(dirs[moveCount % 4]) state=\(st) score=\(g.score) tiles=\(g.tiles.flatten().count)")
            if g.score > bestScore { bestScore = g.score }
        }
        animPending = true
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
    } else if animPending {
        // An animation is in flight: advance the clock by dt, re-evaluate, interpolate the springs.
        // The return value says whether anything is still animating — keep driving fast until it
        // settles, then fall back to idle pacing. (renderFrame picks up @State moves too, since a
        // move sets animPending; its display-list bump is painted by the redraw below.)
        animPending = wandrRenderFrame(dt)
        wandrRedraw()
    } else {
        // Nothing animating: cheap re-walk of the existing display list (double-buffering).
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

// Idle = poll gently (150ms). While an animation is in flight, drive ~30fps so the springs are
// smooth. (60fps continuous GL on WSLg can drop the display connection — "Connection reset by
// peer"; 30fps is the desktop-safe ceiling. Real EGL on device handles faster fine.)
@_cdecl("exports_wandr_ui_shell_frame_pacing_next_frame_delay")
public func nextFrameDelay() -> UInt32 { animPending ? 33 : 150 }

// [PHASE-A .active] track whether a button is held, so pointer MOVES are forwarded into the
// gesture pipeline as .active phases only DURING a press (began → active… → ended) — a real drag
// sequence, not hover. Without this the gesture pipeline only ever sees began → ended.
nonisolated(unsafe) private var pointerPressing = false

@_cdecl("exports_wasi_input_handlers_pointer_handler_on_pointer")
public func onPointer(
    _ ev: UnsafeMutablePointer<exports_wasi_input_handlers_pointer_handler_pointer_event_t>?
) {
    // All gameplay input now flows through real OpenSwiftUI gestures (DragGesture for swipes,
    // .onTapGesture on each control). This handler ONLY forwards host pointer events into the
    // gesture pipeline: down → .began, move-while-pressed → .active, up → .ended.
    guard let e = ev?.pointee else { return }
    switch e.kind {
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_DOWN):
        pointerPressing = true
        wandrSendPointer(phase: 0, x: Double(e.x), y: Double(e.y), serial: ptrSerial)
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_MOVE):
        if pointerPressing {
            wandrSendPointer(phase: 1, x: Double(e.x), y: Double(e.y), serial: ptrSerial)
        }
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_UP):
        pointerPressing = false
        wandrSendPointer(phase: 2, x: Double(e.x), y: Double(e.y), serial: ptrSerial)
        ptrSerial &+= 1
    default: break
    }
}
