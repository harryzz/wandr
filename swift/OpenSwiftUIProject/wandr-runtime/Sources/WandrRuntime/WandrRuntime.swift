//
//  WandrRuntime.swift
//  WandrRuntime
//
//  The shared reactor loop for any OpenSwiftUI-on-wandr app. Per Sources/T2iles/RULES.md, this
//  is GENERIC runtime glue — identical for every app — and must not be duplicated per app:
//  the wasi:canvas embedding handshake (acquire buffer -> render -> present), frame-pacing /
//  animation-driven redraw, and pointer-phase forwarding into OpenSwiftUI's gesture pipeline.
//
//  What CAN'T move here: the four `@_cdecl` WASI export symbols themselves. wit-bindgen-c
//  generates each app's exported-function *parameter types* (e.g. the pointer-event struct)
//  fresh from that app's OWN wit world (its CSwiftSpike), so the exact struct type isn't
//  visible to a sibling package like this one. Each app therefore keeps four ONE-LINE `@_cdecl`
//  forwarding stubs that unwrap its own generated types into plain values and call straight
//  into the functions below — no logic, just the unavoidable per-world type boundary.

import CWASICanvas
import CWandrBoot
// Re-exported so a consumer's single `import WandrRuntime` also gives it CGColor (needed for
// runWandrApp's `background:` argument) without a second explicit import — the same pattern
// OpenCoreGraphicsShims already uses for OpenSwiftUICore.
@_exported import OpenCoreGraphicsWASICanvas
@_spi(WandrRenderer) import OpenSwiftUI
#if canImport(Darwin)
import Darwin
#elseif canImport(Glibc)
import Glibc
#elseif os(WASI)
import WASILibc
#endif

@inline(never) func rlog(_ s: String) { fputs("WANDR-RUNTIME: \(s)\n", stderr); fflush(stderr) }

nonisolated(unsafe) private let sink = CGSink()
nonisolated(unsafe) private var makeAppBox: (() -> Void)?
nonisolated(unsafe) private var willRenderFrameHook: ((_ nanos: UInt64, _ hasBuilt: Bool) -> Void)?
nonisolated(unsafe) private var getDensityHook: (() -> Float)?
nonisolated(unsafe) private var isBusyHook: (() -> Bool)?
nonisolated(unsafe) private var backgroundColor = CGColor(red: 0, green: 0, blue: 0, alpha: 1)

nonisolated(unsafe) private var surfaceWidth: Float = 0
nonisolated(unsafe) private var surfaceHeight: Float = 0
nonisolated(unsafe) private var built = false
nonisolated(unsafe) private var animPending = false
nonisolated(unsafe) private var lastFrameNanos: UInt64 = 0
nonisolated(unsafe) private var density: Float = 1.0
nonisolated(unsafe) private var densityInitialized = false
nonisolated(unsafe) private var pointerPressing = false
nonisolated(unsafe) private var ptrSerial = 0

private func ensureDensity() -> Float {
    if !densityInitialized {
        let d = getDensityHook?() ?? 1.0
        density = d > 0 ? d : 1.0
        densityInitialized = true
    }
    return density
}

/// Register the app + its hooks and prime the reactor. Call ONCE from the app's startup path
/// (e.g. right before returning from its first `on_resize`/`on_frame` forwarding stub is fine —
/// `makeApp` itself isn't invoked until the runtime is ready to build the graph). `makeApp` is
/// called lazily, exactly once, on the first frame the surface has a real size — so an app can
/// defer constructing its own model (GameLogic-style) to that same moment, the same way
/// `renderWandrAppOnce` already defers building the OpenSwiftUI graph.
///
/// - Parameters:
///   - background: cleared behind the DisplayList every frame.
///   - getDensity: returns the display density (guest points-per-pixel); `nil` = no correction
///     (matches un-corrected behavior). Each app supplies its own because the density query is
///     itself a per-app WIT import (`wandr:ui-shell/metrics`), not something this leaf package
///     can reach.
///   - willRenderFrame: called once per frame, before the canvas is acquired, with the host's
///     own frame timestamp and whether the graph has been built yet — the app's own per-frame
///     side effects (audio pump, wall-clock-paced autoplay ticks, ...) belong here. Use `nanos`
///     for any wall-clock pacing rather than querying a clock directly (a second clock source
///     this close to the WASI import boundary is a known footgun on this toolchain). Gate
///     anything that touches OpenSwiftUI state (e.g. `game.move()` via `wandrRuntimeApplyChange`)
///     on `hasBuilt`, since `AppGraph.shared` isn't set before the first real-sized frame.
///   - isBusy: OR'd into the fast/idle frame-pacing decision alongside `animPending` — e.g. an
///     app with in-flight audio wants fast pacing even between OpenSwiftUI animations.
///   - makeApp: builds the app's root `App` value. Called once, lazily.
public func runWandrApp<A: App>(
    background: CGColor = CGColor(red: 0, green: 0, blue: 0, alpha: 1),
    getDensity: (() -> Float)? = nil,
    willRenderFrame: ((_ nanos: UInt64, _ hasBuilt: Bool) -> Void)? = nil,
    isBusy: (() -> Bool)? = nil,
    makeApp: @escaping () -> A
) {
    backgroundColor = background
    getDensityHook = getDensity
    willRenderFrameHook = willRenderFrame
    isBusyHook = isBusy
    makeAppBox = {
        renderWandrAppOnce(
            makeApp(),
            options: .init(surface: CGSize(width: CGFloat(surfaceWidth), height: CGFloat(surfaceHeight)), sink: sink)
        )
    }
}


/// Boot an app that keeps its OWN unmodified `@main struct App` (eleev-style) — no per-app reactor
/// glue. Call ONCE from the guest's `on-init`. Arms the reactor so `App.main()` takes the
/// register-and-return path, runs the @main entry (which calls `registerWandrApp` and returns), then
/// wires the lazy first-real-sized-frame build to the registered launcher (same machinery
/// `runWandrApp` uses, only the app now comes from the registry instead of a passed-in closure).
///
/// - Parameters mirror `runWandrApp`'s ambient ones; the app's root View/Scene, model boot, etc. all
///   come from the app's own `@main` body, so there is no `makeApp`/`willRenderFrame` here.
public func bootWandrReactorApp(
    background: CGColor = CGColor(red: 0, green: 0, blue: 0, alpha: 1),
    getDensity: (() -> Float)? = nil
) {
    backgroundColor = background
    getDensityHook = getDensity
    armWandrReactor()
    _ = wandr_run_app_main()   // C shim → app's __main_argc_argv → App.main() → registerWandrApp → returns
    makeAppBox = {
        launchRegisteredWandrApp(
            options: .init(surface: CGSize(width: CGFloat(surfaceWidth), height: CGFloat(surfaceHeight)), sink: sink)
        )
    }
}

/// Forward a resize. `w`/`h` are RAW PHYSICAL pixels — divided by the app-supplied density (if
/// any) into the logical surface size the graph lays out against.
public func wandrRuntimeOnResize(width w: UInt32, height h: UInt32) {
    let d = ensureDensity()
    surfaceWidth = Float(w) / d
    surfaceHeight = Float(h) / d
}

/// Drive one frame: acquire the wasi:canvas back buffer, build the graph on the first real-sized
/// frame (or re-render/redraw on every frame after), present.
public func wandrRuntimeOnFrame(nanos: UInt64) {
    willRenderFrameHook?(nanos, built)
    // Audio is the library's own concern (WandrAudioPlayer lives here) — drain any queued SFX a bit
    // more each frame as the device consumes the ring, whether or not the app supplied a frame hook.
    WandrAudioPlayer.shared.pump()

    let dt = lastFrameNanos == 0 ? 0.0 : Double(nanos &- lastFrameNanos) / 1_000_000_000.0
    lastFrameNanos = nanos

    // Acquire the context + buffer fresh each frame (caching the `own` handle traps — see
    // repros/swift-canvas-spike/README.md "don't cache the canvas-context own handle").
    let ctxOwn = wasi_canvas_embedding_get_context()
    let ctx = wasi_canvas_embedding_borrow_canvas_context(ctxOwn)
    let bufOwn = wasi_canvas_embedding_method_canvas_context_get_current_buffer(ctx)
    let canvas = wasi_canvas_draw_borrow_canvas(bufOwn)
    let gfxOwn = wasi_canvas_embedding_method_canvas_context_graphics(ctx)
    let gfx = wasi_canvas_draw_borrow_graphics_t(__handle: gfxOwn.__handle)

    let cg = CGContext(canvas: canvas, graphics: gfx)
    cg.clear(backgroundColor)
    // Draw commands are issued in LOGICAL points (already density-divided in onResize) — scale
    // back up so they fill the physical canvas. Fresh CGContext every frame, so this must be
    // re-applied every frame (nothing persists across onFrame calls).
    let d = CGFloat(ensureDensity())
    cg.concat3x3(d, 0, 0, 0, d, 0, 0, 0, 1)
    sink.cg = cg

    if !built, surfaceWidth > 0, surfaceHeight > 0 {
        // The host is retained for the process lifetime; subsequent frames just re-walk the
        // display list (renderWandrAppOnce's own doc comment).
        makeAppBox?()
        built = true
    } else if animPending {
        // An animation is in flight: advance the clock by dt, re-evaluate, interpolate springs.
        animPending = wandrRenderFrame(dt)
        wandrRedraw()
    } else {
        // Nothing animating: cheap re-walk of the existing display list (double-buffering).
        wandrRedraw()
    }
    sink.cg = nil

    wasi_canvas_draw_canvas_drop_own(bufOwn)
    wasi_canvas_embedding_method_canvas_context_present(ctx)
    wasi_canvas_draw_graphics_drop_own(wasi_canvas_draw_own_graphics_t(__handle: gfxOwn.__handle))
    wasi_canvas_embedding_canvas_context_drop_own(ctxOwn)
}

/// Idle = poll gently; while an animation is in flight (or the app reports `isBusy`), drive
/// faster so springs interpolate smoothly. 33ms/150ms mirrors the tuning already verified across
/// both prior per-app copies.
public func wandrRuntimeNextFrameDelay() -> UInt32 {
    (animPending || WandrAudioPlayer.shared.isActive || (isBusyHook?() ?? false)) ? 33 : 150
}

/// Forward one raw pointer event, in the SAME raw physical-pixel space `onResize` receives its
/// `w`/`h` in — divided here by density to match the logical points OpenSwiftUI laid out
/// against, so hit-testing lines up with where views actually are. `phase`: `0` = down,
/// `1` = move, `2` = up, anything else = cancel — matches `wandrSendPointer`'s own convention.
/// Only forwards MOVE while pressed (hover isn't a drag), and bumps the interaction serial on UP
/// so the next press starts a fresh one.
public func wandrRuntimeOnPointer(phase: Int, x rawX: Double, y rawY: Double) {
    let d = Double(ensureDensity())
    let x = rawX / d, y = rawY / d
    switch phase {
    case 0:
        pointerPressing = true
        wandrSendPointer(phase: 0, x: x, y: y, serial: ptrSerial)
    case 1:
        if pointerPressing {
            wandrSendPointer(phase: 1, x: x, y: y, serial: ptrSerial)
        }
    case 2:
        pointerPressing = false
        wandrSendPointer(phase: 2, x: x, y: y, serial: ptrSerial)
        ptrSerial &+= 1
    default:
        wandrSendPointer(phase: 3, x: x, y: y, serial: ptrSerial)
    }
    // A gesture action may have mutated state and/or scheduled a `withAnimation` transition —
    // kick the animation loop so onFrame re-runs the graph and ADVANCES the clock via
    // wandrRenderFrame, otherwise the steady-state redraw-only path leaves animated properties
    // frozen at their old values.
    animPending = true
}

/// Apply a state mutation (e.g. from a raw reactor input handler) inside an OpenSwiftUI update
/// transaction and mark the next frame as animating, so the change actually gets picked up and
/// drawn. Thin re-export of `wandrApplyChange` + the runtime's own `animPending` bookkeeping, so
/// an app doesn't need its own copy of either.
public func wandrRuntimeApplyChange(_ body: () -> Void) {
    wandrApplyChange(body)
    animPending = true
}
