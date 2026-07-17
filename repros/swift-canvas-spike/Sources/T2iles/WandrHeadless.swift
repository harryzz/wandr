// [wandr TEMPORARY — remove once the teardown UAF is fixed & verified]
// Deterministic HEADLESS driver for the real eleev CompositeView, gated behind -DWANDR_HEADLESS.
// Purpose: reproduce the intermittent AttributeGraph teardown crash (crash "site 5":
// StoredLocation.notifyObservers -> value_mark -> propagate_dirty) DETERMINISTICALLY, with NO host,
// NO wasi:canvas, NO graphics — driving the SAME transaction model the device runs, via the OpenSwiftUI
// stdout/WandrRenderer path. A pass/crash here is binary and repeatable (no manual play, no luck), so a
// candidate fix can be judged instantly. Uses the app's OWN CompositeView (WandrHostApp) so it is faithful
// to the deployed view tree; it only scripts the navigation the user drives by hand.
#if WANDR_HEADLESS
import CSwiftSpike  // pointer_event_t type for the inert export stubs (no wasi:canvas calls)
import OpenCoreGraphicsWASICanvas  // CGSize (value type; avoids the WASICanvas-backed CGContext code)
import SwiftUI       // apple-compat shim (re-exports OpenSwiftUI types the eleev views are written against)
import Combine       // ObservableObject / @Published (apple-compat)
import OpenSwiftUI
@_spi(WandrRenderer) import OpenSwiftUI

// The wit-bindgen-c reactor glue (swift_spike.c) hard-references these export symbols. The real
// implementations live in WandrReactor.swift and touch wasi:canvas, so they're gated out under
// WANDR_HEADLESS. Provide INERT stubs so the plain command links WITHOUT importing wasi:canvas.
@_cdecl("exports_wasi_input_handlers_frame_handler_on_resize")
public func hlOnResize(_ w: UInt32, _ h: UInt32) {}
// This reactor export IS retained (the wit-bindgen C wrapper references it via __export_name__), so we
// launch the deterministic driver from here on the first call — no linker --export flag needed. Invoke:
//   wasmtime run --invoke 'wasi:input-handlers/frame-handler@0.0.2#on-frame' ... <module> 0
@_cdecl("exports_wasi_input_handlers_frame_handler_on_frame")
public func hlOnFrame(_ nanos: UInt64) {
    if hlStarted { return }
    hlStarted = true
    wandrHeadlessRun()
}
nonisolated(unsafe) var hlStarted = false
@_cdecl("exports_wasi_input_handlers_pointer_handler_on_pointer")
public func hlOnPointer(_ ev: UnsafeMutablePointer<exports_wasi_input_handlers_pointer_handler_pointer_event_t>?) {}
@_cdecl("exports_wandr_ui_shell_frame_pacing_next_frame_delay")
public func hlNextFrameDelay() -> UInt32 { 0 }
#if canImport(WASILibc)
import WASILibc
#endif

@inline(never) func hlog(_ s: String) { fputs("HEADLESS: \(s)\n", stderr); fflush(stderr) }

// A canvas-free WandrDrawSink: records item frames + text so we can (a) confirm the render happened and
// (b) locate on-screen tap targets (hamburger, "Settings" row) for scripted pointer injection.
final class HeadlessSink: WandrDrawSink {
    nonisolated(unsafe) static var texts: [(String, Double, Double, Double, Double)] = []
    nonisolated(unsafe) static var rectCount = 0
    func beginFrame(width: Double, height: Double, version: UInt32) { Self.texts = []; Self.rectCount = 0 }
    func fillRect(x: Double, y: Double, width: Double, height: Double,
                  red: Float, green: Float, blue: Float, opacity: Float) { Self.rectCount += 1 }
    func drawText(_ text: String, x: Double, y: Double, width: Double, height: Double,
                  fontSize: Double, red: Float, green: Float, blue: Float, opacity: Float,
                  fontFamily: String) {
        if !text.isEmpty { Self.texts.append((text, x, y, width, height)) }
    }
    func fillPath(svgPath: String, x: Double, y: Double, width: Double, height: Double,
                  red: Float, green: Float, blue: Float, opacity: Float) { Self.rectCount += 1 }
    func pushClip(svgPath: String) {}
    func popClip() {}
    var wandrSupportsProjection: Bool { true }
    func saveState() {}
    func restoreState() {}
    func concat(m00: Double, m01: Double, m02: Double,
                m10: Double, m11: Double, m12: Double,
                m20: Double, m21: Double, m22: Double) {}
    func fillPathShadow(svgPath: String, dx: Double, dy: Double, blur: Double,
                        red: Float, green: Float, blue: Float, opacity: Float) {}
    func endFrame() {}
}

// Advance the graph a few frames so any withAnimation transition (menu slide, screen swap) settles and
// its teardown actually runs — the teardown, not the initial build, is where the UAF lives.
@inline(never) func settle(_ frames: Int = 8, dt: Double = 0.016) {
    for _ in 0..<frames { _ = wandrRenderFrame(dt) }
    wandrRedraw()
}

@inline(never) func dumpTexts(_ tag: String) {
    hlog("\(tag): rects=\(HeadlessSink.rectCount) texts=\(HeadlessSink.texts.count)")
    for (t, x, y, w, h) in HeadlessSink.texts.prefix(24) {
        hlog("  TEXT '\(t)' @\(Int(x)),\(Int(y)) \(Int(w))x\(Int(h))")
    }
}

// Inject a full down→up tap at a surface point via the REAL input shim (wandrSendPointer), exactly like
// the host's on_pointer path. Then settle so the gesture action drains through the transaction.
@inline(never) func tap(_ x: Double, _ y: Double, _ frames: Int = 4) {
    wandrSendPointer(phase: 0, x: x, y: y, serial: ptrSerialH)
    wandrSendPointer(phase: 2, x: x, y: y, serial: ptrSerialH)
    ptrSerialH &+= 1
    settle(frames)
}
nonisolated(unsafe) var ptrSerialH = 0

// The T2iles product is a wasip1 REACTOR (-mexec-model=reactor), so there is no _start/main. The driver
// is launched from the (retained) on-frame export; see hlOnFrame.
@_cdecl("wandr_headless_run")
public func wandrHeadlessRun() {
        print("HEADLESS-START"); fflush(stdout)
        let W = 460.0, H = 920.0
        sharedGame = GameLogic(size: BoardSize.fourByFour.rawValue)
        let sink = HeadlessSink()

        hlog("build graph (REAL CompositeView / WandrHostApp) \(Int(W))x\(Int(H))")
        renderWandrAppOnce(WandrHostApp(),
            options: .init(surface: CGSize(width: W, height: H), sink: sink))
        settle()
        dumpTexts("INITIAL selected=.game")

        // Find an ON-SCREEN side-menu row by label (y>300 => below the header; x>0 => slid in).
        func menuRow(_ label: String) -> (Double, Double)? {
            for (t, x, y, w, h) in HeadlessSink.texts where t == label && y > 300 && x > 0 {
                return (x + w/2, y + h/2)
            }
            return nil
        }
        func headerTitle() -> String {
            // The header title text sits near the top (y<130); returns "T2iles"/"Settings"/"About".
            for (t, _, y, _, _) in HeadlessSink.texts where y < 130 && (t == "T2iles" || t == "Settings" || t == "About") { return t }
            return "?"
        }

        // Drive the REAL reported interaction entirely through the input shim (no state hacks): open the
        // side menu (hamburger), tap Settings, reopen, tap the game row — the real CompositeSideView /
        // FactoryContentView switch / modals all tear down & rebuild exactly as on device. The menu slides
        // via .modalSpring, so settle generously (~28 frames) for rows to reach their on-screen x.
        _ = menuRow
        // Text WIDTHS are garbage headless (~100000, no font metrics), so tap near a row's LEFT EDGE.
        func menuRowLeft(_ label: String) -> (Double, Double)? {
            for (t, x, y, _, h) in HeadlessSink.texts where t == label && y > 300 && x > 0 {
                return (x + 20, y + h / 2)
            }
            return nil
        }
        // Find an ON-SCREEN (y within surface) label's left-edge tap point.
        func onScreen(_ label: String) -> (Double, Double)? {
            for (t, x, y, _, h) in HeadlessSink.texts where t == label && y >= 0 && y <= H && x > 0 {
                return (x + 20, y + h / 2)
            }
            return nil
        }
        // One real swipe over the board (DragGesture > 25pt threshold -> logic.move) to grow the graph.
        func swipe(_ dx: Double, _ dy: Double) {
            let cx = 230.0, cy = 450.0
            wandrSendPointer(phase: 0, x: cx, y: cy, serial: ptrSerialH)
            wandrSendPointer(phase: 1, x: cx + dx, y: cy + dy, serial: ptrSerialH)
            wandrSendPointer(phase: 2, x: cx + dx, y: cy + dy, serial: ptrSerialH)
            ptrSerialH &+= 1
            settle(8)
        }

        // FULL interaction, faithful to the reported crash ("reset the game -> tap Settings"): play a few
        // moves, RESET (new-game button -> Ok), then cycle Settings/About/game. All via the real input shim
        // on the real CompositeView. Deterministic: traps (UAF reproduced) or survives.
        let targets = ["Settings", "About", "T2iles"]
        let rounds = 60   // well past the round-0..1 crash point; a full survive => the fix holds
        for round in 1...rounds {
            // 1) grow the board with real swipes
            swipe(-70, 0); swipe(0, -70); swipe(70, 0); swipe(0, 70)
            // 2) reset the game: header new-game button (right, ~420,88) -> modal slides up -> tap Ok
            tap(420, 88, 20)
            let ok = onScreen("Ok")
            if let (ox, oy) = ok { tap(ox, oy, 20) }
            // 3) navigate every screen (the teardown the crash rides on)
            for target in targets {
                tap(40, 88, 36)
                if let (mx, my) = menuRowLeft(target) { tap(mx, my, 36) }
            }
            if round <= 2 {
                hlog("  r\(round): okFound=\(ok != nil) header=\(headerTitle())")
                for (t, x, y, _, _) in HeadlessSink.texts where t == "Ok" || t == "Cancel" { hlog("    modalbtn '\(t)' @\(Int(x)),\(Int(y))") }
            }
            if round % 5 == 0 { print("round \(round) OK header=\(headerTitle())"); fflush(stdout) }
        }
        print("HEADLESS-SURVIVED all \(rounds) rounds (moves + reset + navigation)"); fflush(stdout)
}
#endif
