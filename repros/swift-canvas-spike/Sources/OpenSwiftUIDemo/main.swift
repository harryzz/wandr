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
                    // [PHASE-A PROBE] the ONLY .onTapGesture in the tree — structural binding ignores
                    // location, so any down→up routed via wandrSendPointer should fire this.
                    .onTapGesture { tapCount &+= 1; wlog("TAP-FIRED count=\(tapCount)") }
                ZStack {
                    TileBoardView(
                        matrix: game.tiles,     // read in BODY (game fully constructed), not in init
                        tileEdge: game.lastGestureDirection.invertedEdge,
                        tileBoardSize: game.boardSize
                    )
                    // Game over (board full, no merges left) → dim + prompt. Any tap restarts (onPointer).
                    if !game.tiles.isMovePossible() {
                        Rectangle().fill(Color(red: 0.05, green: 0.06, blue: 0.07, opacity: 0.72))
                        VStack(spacing: w * 0.025) {
                            Text("GAME OVER")
                                .font(.system(size: w * 0.10, weight: .heavy)).foregroundColor(.white)
                            Text("tap to start a new game")
                                .font(.system(size: w * 0.040, weight: .medium)).foregroundColor(labelColor)
                        }
                    }
                }
            }
            // New-game confirmation (tapping "2048" opens this; nothing resets without a YES).
            if confirmNewGame {
                Rectangle().fill(Color(red: 0.05, green: 0.06, blue: 0.07, opacity: 0.80))
                VStack(spacing: w * 0.06) {
                    Text("New game?").font(.system(size: w * 0.085, weight: .heavy)).foregroundColor(.white)
                    HStack(spacing: w * 0.07) {
                        confirmButton("NO", noColor, w)
                        confirmButton("YES", yesColor, w)
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
        // [wandr no-hardcode input] recover where the board + score boxes draw, by their unique
        // background fill colors (surface units, same space as pointer events). These literals mirror
        // TileBoardView's board background and ContentView.boxColor.
        func near(_ a: Float, _ b: Float) -> Bool { abs(a - b) < 0.02 }
        if near(red, 0.76), near(green, 0.76), near(blue, 0.78) {        // board outer background
            boardRect = (Float(x), Float(y), Float(width), Float(height))
        } else if near(red, 0.73), near(green, 0.68), near(blue, 0.63) { // SCORE / BEST box background
            let bx = Float(x), by = Float(y), bw = Float(width), bh = Float(height)
            if scoreRect.h == 0 {
                scoreRect = (bx, by, bw, bh)
            } else {                                                    // union with the other box
                let minX = min(scoreRect.x, bx), minY = min(scoreRect.y, by)
                let maxX = max(scoreRect.x + scoreRect.w, bx + bw), maxY = max(scoreRect.y + scoreRect.h, by + bh)
                scoreRect = (minX, minY, maxX - minX, maxY - minY)
            }
        } else if near(red, 0.30), near(green, 0.69), near(blue, 0.31) { // confirm-dialog YES button
            yesRect = (Float(x), Float(y), Float(width), Float(height))
        } else if near(red, 0.35), near(green, 0.38), near(blue, 0.45) { // confirm-dialog NO button
            noRect = (Float(x), Float(y), Float(width), Float(height))
        }
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
        // [wandr no-hardcode input] the "2048" title is text (no fill) — capture its rect so only
        // the title (not the hint label beside it) opens the new-game dialog.
        if text == "2048" { titleRect = (Float(x), Float(y), Float(width), Float(height)) }
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
// Input regions DERIVED from where the elements actually draw (no hardcoded positions): the CGSink
// recovers each element's rect by its unique background fill color, in SURFACE units (the same space
// as the pointer events). `boardRect` = the board square (swipe area); `scoreRect` = the union of the
// SCORE/BEST boxes (autoplay-toggle area). A tap ABOVE the board and LEFT of the score boxes is the
// "2048" title (new game). Tracks the real layout on any device/orientation. (Plain vars — a lazy
// global `let` reads 0 on this wasm toolchain, and the match colors are inlined in fillRect for the
// same reason; they mirror TileBoardView's board background and ContentView.boxColor.)
nonisolated(unsafe) private var boardRect: (x: Float, y: Float, w: Float, h: Float) = (0, 0, 0, 0)
nonisolated(unsafe) private var scoreRect: (x: Float, y: Float, w: Float, h: Float) = (0, 0, 0, 0)
nonisolated(unsafe) private var titleRect: (x: Float, y: Float, w: Float, h: Float) = (0, 0, 0, 0)  // "2048" text
nonisolated(unsafe) private var yesRect:   (x: Float, y: Float, w: Float, h: Float) = (0, 0, 0, 0)  // confirm YES
nonisolated(unsafe) private var noRect:    (x: Float, y: Float, w: Float, h: Float) = (0, 0, 0, 0)  // confirm NO
nonisolated(unsafe) private var confirmNewGame = false  // show the "New game?" yes/no dialog
// [PHASE-A PROBE] does a tap routed through OpenSwiftUI's gesture pipeline fire a .onTapGesture?
nonisolated(unsafe) private var tapCount = 0
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

// Swipe → GameLogic.move (down=0 start, up=1 release; dominant axis/sign picks direction).
nonisolated(unsafe) private var swipeStartX: Float = 0
nonisolated(unsafe) private var swipeStartY: Float = 0
// [PHASE-A .active] track whether a button is held, so pointer MOVES are forwarded into the
// gesture pipeline as .active phases only DURING a press (began → active… → ended) — a real drag
// sequence, not hover. Without this the gesture pipeline only ever sees began → ended.
nonisolated(unsafe) private var pointerPressing = false

@_cdecl("exports_wasi_input_handlers_pointer_handler_on_pointer")
public func onPointer(
    _ ev: UnsafeMutablePointer<exports_wasi_input_handlers_pointer_handler_pointer_event_t>?
) {
    guard let e = ev?.pointee else { return }
    switch e.kind {
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_DOWN):
        swipeStartX = e.x; swipeStartY = e.y
        pointerPressing = true
        wandrSendPointer(phase: 0, x: Double(e.x), y: Double(e.y), serial: ptrSerial)  // [PHASE-A PROBE]
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_MOVE):
        // [PHASE-A .active] forward moves DURING a press so OpenSwiftUI gestures receive .active
        // phases (DragGesture). Additive: the hand-rolled swipe still uses the down→up delta.
        if pointerPressing {
            wandrSendPointer(phase: 1, x: Double(e.x), y: Double(e.y), serial: ptrSerial)
        }
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_UP):
        pointerPressing = false
        wandrSendPointer(phase: 2, x: Double(e.x), y: Double(e.y), serial: ptrSerial)  // [PHASE-A PROBE]
        ptrSerial &+= 1
        let dx = e.x - swipeStartX, dy = e.y - swipeStartY
        let t: Float = 24
        guard let game = sharedGame, boardRect.h > 0 else { break }   // board rect not captured yet
        let px = swipeStartX, py = swipeStartY
        func inRect(_ r: (x: Float, y: Float, w: Float, h: Float)) -> Bool {
            r.h > 0 && px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
        }

        // Confirmation dialog is up → only its YES / NO buttons respond.
        if confirmNewGame {
            if inRect(yesRect) {
                autoplayOn = false
                wandrApplyChange { game.reset(); confirmNewGame = false; tickBinding?.wrappedValue &+= 1 }
                animPending = true
            } else if inRect(noRect) {
                wandrApplyChange { confirmNewGame = false; tickBinding?.wrappedValue &+= 1 }
                animPending = true
            }
            break
        }

        // Game over → any release anywhere starts a new game (already a loss — no confirm needed).
        if !game.tiles.isMovePossible() {
            autoplayOn = false
            wandrApplyChange { game.reset(); tickBinding?.wrappedValue &+= 1 }
            animPending = true
            break
        }

        // ABOVE THE BOARD = a header press (no movement threshold). Tap the SCORE/BEST boxes → toggle
        // autoplay; tap the "2048" title → open the new-game confirm dialog. The hint label between
        // them is NOT a control (neither rect contains it). All rects are where the elements drew.
        if py < boardRect.y {
            if inRect(scoreRect) {
                wandrApplyChange { autoplayOn.toggle(); tickBinding?.wrappedValue &+= 1 }
                animPending = true
            } else if inRect(titleRect) {
                wandrApplyChange { confirmNewGame = true; tickBinding?.wrappedValue &+= 1 }
                animPending = true
            }
            break
        }

        // SWIPE — only if it STARTED inside the board square (the empty area below the board and the
        // header are excluded). Manual play takes control, so autoplay turns off.
        guard inRect(boardRect) else { break }
        let dir: Direction?
        if abs(dx) > abs(dy) {
            dir = dx > t ? .right : (dx < -t ? .left : nil)
        } else {
            dir = dy > t ? .down : (dy < -t ? .up : nil)
        }
        if let dir {
            autoplayOn = false
            wandrApplyChange {
                _ = game.move(dir)
                tickBinding?.wrappedValue &+= 1
            }
            if game.score > bestScore { bestScore = game.score }
            animPending = true   // kick off the spring; onFrame drives it to settle
        }
    default: break
    }
}
