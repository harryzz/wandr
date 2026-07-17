// Phase 4b — OpenSwiftUI renders on wandr THROUGH the shared WandrRuntime reactor loop (wasi:canvas
// embedding handshake, frame pacing, pointer forwarding — see that package's own doc comment).
// The OpenSwiftUI engine (AttributeGraph/Compute on wasm) lays out a real SwiftUI view, emits
// a DisplayList, and WandrRuntime's CGSink walks it into a CGContext (OpenCoreGraphics over
// wasi:canvas). Same frame plumbing as the hand-built spike (spike.swift) — only the draw
// source changed (SwiftUI instead of hand-coded).
import CSwiftSpike
import WandrRuntime
import OpenSwiftUI
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
    @ObservedObject var game: GameLogic // [VERIFY] real @ObservedObject — subscribes to game.objectWillChange
    @State private var tick = 0         // re-render trigger; on_pointer bumps it after a move

    private var titleColor: Color { Color(red: 0.47, green: 0.43, blue: 0.40, opacity: 1) }
    private var labelColor: Color { Color(red: 0.93, green: 0.89, blue: 0.85, opacity: 1) }
    private var boxColor:   Color { Color(red: 0.73, green: 0.68, blue: 0.63, opacity: 1) }
    private var hintColor:  Color { Color(red: 0.58, green: 0.56, blue: 0.53, opacity: 1) }

    // A score/best readout box (mirrors eleev's ScoreView header).
    private func scoreBox(_ label: String, _ value: Int, _ w: CGFloat, multiplier: Int = 0) -> some View {
        // "Dancing score" — ported straight from eleev's HeaderView.scoreView using the SAME .id-swap
        // transition our TileView already uses for tile numbers (TileView.swift:57-59): a fresh .id on
        // each score change swaps the number, which slides up from the bottom + fades in
        // (.move(.bottom)+.opacity, sprung). No scaleEffect (that grew the number out of the box to the
        // right). The box is .clipped() so the roll stays inside. Colors stay legible white/cream; the
        // live combo shows as "xN" in the label. BEST passes multiplier 0 (no badge).
        let combo = multiplier > 0
        return VStack(spacing: 0) {
            HStack(spacing: w * 0.012) {
                Text(label).font(.system(size: w * 0.026, weight: .medium)).foregroundColor(labelColor)
                if combo {
                    Text("x\(multiplier)").font(.system(size: w * 0.026, weight: .heavy)).foregroundColor(labelColor)
                }
            }
            Text("\(value)")
                .font(.system(size: w * 0.050, weight: .heavy))
                .id(value)
                .foregroundColor(.white)
                .transition(AnyTransition.move(edge: .bottom).combined(with: .opacity).animation(.modalSpring(duration: 0.3)))
        }
        .frame(width: w * 0.25, height: w * 0.14)
        .background(Rectangle().fill(boxColor).cornerRadius(w * 0.012))
        .clipped()
        // Real gesture: tap a score box → toggle autoplay (replaces the hand-rolled scoreRect hit-test).
        .onTapGesture {
            wandrRuntimeApplyChange { autoplayOn.toggle(); tickBinding?.wrappedValue &+= 1 }
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
                            wandrRuntimeApplyChange { confirmNewGame = true; tickBinding?.wrappedValue &+= 1 }
                        }
                    Spacer()
                    scoreBox("SCORE", game.score, w, multiplier: game.mergeMultiplier)
                    scoreBox("BEST", max(bestScore, game.score), w)
                }
                .frame(height: w * 0.16)
                // Mode hint + how to reach the autoplay/new-game controls (the top band, see onPointer).
                Text(autoplayOn ? "AUTO ON  -  tap 2048 = new game  -  tap a score box = stop"
                                : "swipe to play  -  tap a score box = autoplay")
                    .font(.system(size: w * 0.032, weight: .medium))
                    .foregroundColor(hintColor)
                ZStack {
                    // The swipe gesture now rides on the DRAWN board square INSIDE TileBoardView (so
                    // Approach-A binds its hit-frame to the tiles, not the greedy GeometryReader — a
                    // swipe in the empty band above/below the board no longer moves tiles). onSwipe gets
                    // the drag translation; the move / game-over-restart logic stays here.
                    TileBoardView(
                        matrix: game.tiles,     // read in BODY (game fully constructed), not in init
                        tileEdge: game.lastGestureDirection.invertedEdge,
                        tileBoardSize: game.boardSize,
                        onSwipe: { t in
                            guard !confirmNewGame else { return }
                            if !game.tiles.isMovePossible() {
                                wandrRuntimeApplyChange { autoplayOn = false; game.reset(); tickBinding?.wrappedValue &+= 1 }
                                return
                            }
                            let dir: Direction = abs(t.width) > abs(t.height)
                                ? (t.width > 0 ? .right : .left)
                                : (t.height > 0 ? .down : .up)
                            wandrRuntimeApplyChange { autoplayOn = false; _ = game.move(dir) }  // @ObservedObject re-renders (move fires objectWillChange)
                            if game.score > bestScore { bestScore = game.score }
                        }
                    )
                    // Game over (board full, no merges left) → dim + prompt. A swipe ON the board square
                    // restarts (handled by onSwipe above); the dim overlay carries no gesture so it does
                    // not block the board's gesture beneath it.
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
                                wandrRuntimeApplyChange { confirmNewGame = false; tickBinding?.wrappedValue &+= 1 }
                            }
                        confirmButton("YES", yesColor, w)
                            .onTapGesture {
                                wandrRuntimeApplyChange { autoplayOn = false; game.reset(); confirmNewGame = false; tickBinding?.wrappedValue &+= 1 }
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

// MARK: - Reactor glue — see WandrRuntime's own doc comment for why these four `@_cdecl` stubs
// can't move into the shared package (each app's exported-function parameter types are
// generated fresh from its own wit world). Everything else — the WandrDrawSink conformer, the
// wasi:canvas handshake, frame pacing, pointer forwarding — lives in WandrRuntime.

nonisolated(unsafe) private var moveCount = 0        // auto-play move number (wall-clock paced)
nonisolated(unsafe) private var lastMoveNanos: UInt64 = 0   // wall-clock of the last auto-move
nonisolated(unsafe) private var autoplayOn = false  // demo autoplay — DEFAULT OFF (user plays by swiping)
nonisolated(unsafe) private var bestScore = 0       // peak score across the session (header "BEST" box)
nonisolated(unsafe) private var confirmNewGame = false  // show the "New game?" yes/no dialog

private func bootRuntime() {
    runWandrApp(
        background: CGColor(red: 0.063, green: 0.078, blue: 0.094, alpha: 1),
        willRenderFrame: { nanos, hasBuilt in
            // Construct the game HERE, in the plain reactor context, before the graph is built —
            // so GameLogic.init (→ reset → tileMatrix) runs outside the OpenSwiftUI graph eval.
            if sharedGame == nil { sharedGame = GameLogic(size: 4) }

            // Autoplay — DEFAULT OFF (toggled from the header, see the score-box tap gesture).
            // One move every 450ms, wall-clock paced (using the host's own frame timestamp, not
            // a second clock source) and DECOUPLED from the frame rate so the spring animates
            // smoothly between moves (a frame-count cadence would speed the game up whenever the
            // frames sped up). Gated on `hasBuilt` — the first move needs the graph's subgraph
            // (AppGraph.shared) set up.
            guard hasBuilt, autoplayOn, let g = sharedGame else { return }
            guard lastMoveNanos == 0 || nanos &- lastMoveNanos >= 450_000_000 else { return }
            lastMoveNanos = nanos
            if !g.tiles.isMovePossible() {
                // Board stuck (game over) → restart so autoplay keeps running.
                wandrRuntimeApplyChange { g.reset(); tickBinding?.wrappedValue &+= 1 }
            } else {
                moveCount &+= 1
                let dirs: [Direction] = [.up, .left, .down, .right]
                var st: GameLogic.State = .none
                // [VERIFY reactivity] NO tick bump — rely on @ObservedObject: g.move() fires
                // objectWillChange.send() → @ObservedObject invalidates → renderFrame re-evals
                // the board.
                wandrRuntimeApplyChange { st = g.move(dirs[moveCount % 4]) }
                wlog("AUTO n=\(moveCount) dir=\(dirs[moveCount % 4]) state=\(st) score=\(g.score) tiles=\(g.tiles.flatten().count)")
                if g.score > bestScore { bestScore = g.score }
            }
        },
        makeApp: { DemoApp() }
    )
}
nonisolated(unsafe) private var runtimeBooted = false

@_cdecl("exports_wasi_input_handlers_frame_handler_on_resize")
public func onResize(_ w: UInt32, _ h: UInt32) {
    wandrRuntimeOnResize(width: w, height: h)
}

@_cdecl("exports_wasi_input_handlers_frame_handler_on_frame")
public func onFrame(_ nanos: UInt64) {
    if !runtimeBooted { bootRuntime(); runtimeBooted = true }
    wandrRuntimeOnFrame(nanos: nanos)
}

@_cdecl("exports_wandr_ui_shell_frame_pacing_next_frame_delay")
public func nextFrameDelay() -> UInt32 { wandrRuntimeNextFrameDelay() }

@_cdecl("exports_wasi_input_handlers_pointer_handler_on_pointer")
public func onPointer(_ ev: UnsafeMutablePointer<exports_wasi_input_handlers_pointer_handler_pointer_event_t>?) {
    guard let e = ev?.pointee else { return }
    let phase: Int
    switch e.kind {
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_DOWN): phase = 0
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_MOVE): phase = 1
    case UInt8(EXPORTS_WASI_INPUT_HANDLERS_POINTER_HANDLER_KIND_UP): phase = 2
    default: phase = 3
    }
    wandrRuntimeOnPointer(phase: phase, x: Double(e.x), y: Double(e.y))
}

// This demo has no audio, but wit/spike.wit (shared with T2iles) exports shell-events — the
// wit-bindgen-c reactor glue hard-references these symbols regardless, so provide inert stubs.
@_cdecl("exports_wandr_ui_shell_shell_events_on_scheduled_callback")
public func onScheduledCallback(_ callbackId: UInt32) {}

@_cdecl("exports_wandr_ui_shell_shell_events_on_lifecycle_changed")
public func onLifecycleChanged(_ newState: exports_wandr_ui_shell_shell_events_state_t) {}
