# The surface/graphics-context socket — design record + upstream feedback (NOT WIRED, NOT OURS)

> Design record only (2026-06-12, ownership framing corrected same
> day). Bound by the same user-fixed GOAL and rules (R1–R5,
> `../wasi-canvas/REDESIGN-0.0.2.md` §2); de-floats the socket model
> from `docs/skiko-gfx-vs-wasi-gfx.md` + the convergence doc.
>
> **Ownership: `wasi:surface` and `wasi:graphics-context` are
> UPSTREAM-OWNED packages (the wasi-gfx org), unlike our greenfield
> wasi:canvas / wasi:input-handlers drafts.** The WIT below is therefore
> NOT a wandr package and claims no version lineage — it is the
> CHANGE-SET we would propose upstream (capability-granted context,
> request/configure, configure-with-scale, optional pull events), with
> version labels purely illustrative for validation. If wandr ever
> needs to SHIP this layer before upstream stabilizes, it ships under
> OUR namespace as `wandr:surface@0.0.1` and migrates when upstream
> lands — the my:skiko-gfx precedent. What wandr's own contracts
> actually depend on is only each producer's connection lane, and those
> live in OUR packages (canvas's un-fuse lane, video's factoring lane)."

## The model (the one sentence)

**Everything renders through the same socket; consumers differ only in
who fills the buffer.**

```
wasi:surface          — WHERE pixels go: geometry, configure events
      │ get-context()
wasi:graphics-context — THE SOCKET: get-current-buffer / present
      │
      ├─ webgpu device        guest GPU rendering        (upstream)
      ├─ frame-buffer device  guest raw pixels           (upstream)
      ├─ canvas               guest 2D semantics,        (wasi:canvas —
      │                       host rasterizes              the third type)
      └─ video decoder        host codec fills buffers   (wandr:video —
                                the "media element", fourth type)
```

The web platform ships exactly this shape (one canvas/surface; 2D,
WebGPU and media sinks behind it); so does every compositor stack.

## The shapes (wasm-tools-validated 2026-06-12)

> Reproducible artifact: `wit-sketch/` beside this doc — the same shapes
> as files, headed with the upstream-ownership disclaimer (validation
> material for the change-set, never a shipped package).

```wit
package wasi:graphics-context@0.0.2;

/// The swapchain SOCKET between one surface and one buffer producer.
interface graphics-context {
    /// Obtained FROM a surface (capability-granted) — no ambient
    /// constructor (deliberate divergence from upstream 0.0.1, whose
    /// free-floating `constructor()` violates WASI's own
    /// no-ambient-authority rule).
    resource context {
        /// Acquire the current frame's buffer; the connected producer
        /// package converts it (the from-graphics-buffer idiom).
        get-current-buffer: func() -> abstract-buffer;
        present: func();
    }
    /// Opaque until converted by the connected producer.
    resource abstract-buffer;
}
```

```wit
package wasi:surface@0.0.2;

interface surface {
    use wasi:graphics-context/graphics-context@0.0.2.{context};

    /// xdg_shell model: clients REQUEST, compositors CONFIGURE.
    record configure-event {
        width:  u32,
        height: u32,
        /// Device scale (1.0 = 1:1); changes when a desktop window
        /// crosses monitors.
        scale:  f32,
    }

    resource surface {
        /// No constructor: surfaces are EMBEDDER-GRANTED (a reactor
        /// guest's primary surface via its world entry; child surfaces
        /// via embedder policy — wandr: the arbiter).
        width:  func() -> u32;
        height: func() -> u32;
        scale:  func() -> f32;
        /// Advisory; the embedder MAY ignore it. Authoritative size
        /// always arrives as a configure.
        request-set-size: func(width: option<u32>, height: option<u32>);
        /// The surface's swapchain socket — one per surface, stable.
        get-context: func() -> context;
    }
}

/// OPTIONAL pull-profile events for poll-loop guests. Reactor guests
/// use wasi:input-handlers (push); the embedder routes EXCLUSIVELY —
/// the same no-double-delivery rule as keys-vs-IME.
interface surface-events {
    use wasi:io/poll@0.2.0.{pollable};
    use surface.{surface, configure-event};
    subscribe-configure: func(s: borrow<surface>) -> pollable;
    get-configure:       func(s: borrow<surface>) -> option<configure-event>;
}
```

## How each producer connects (the four types)

| Producer | Connection idiom | Status |
|---|---|---|
| webgpu / frame-buffer | upstream's `device.connect-graphics-context(borrow<context>)` + `buffer.from-graphics-buffer(abstract-buffer)` — referenced, not redesigned here | upstream, pre-stable |
| **canvas** (third type) | DRAFTED: `wasi:canvas@0.0.2/connection` (`proposals/wasi-canvas/wit-0.0.2/connection.wit`, validated) — `canvas-device.connect(ctx)` + `from-graphics-buffer`; NOT in the canvas-host world yet. The fused `embedding.canvas-context` stays the reactor form (documented equivalence) | both forms now artifacts; un-fused wires at the trigger |
| **video decoder** (fourth type) | DRAFTED: `proposals/wasi-video-decoder/` — decoder + `connect(ctx)`; placement/visibility/z moved to surface vocabulary; CVO `set-rotation` kept (frame property, not placement) | draft validated 2026-06-12; fused wandr:video keeps shipping side-by-side until the wiring trigger |

## Event-profile tie-in (no new vocabulary)

The push profile already exists: `wasi:input-handlers@0.0.2` —
`frame-handler.on-resize` IS configure delivery (scale changes arrive
as a resize + metrics re-query today; an atomic `on-configure` with
scale is a named lane for input-handlers 0.0.3 if a consumer ever needs
atomicity). The pull profile is `surface-events` above. One vocabulary,
two deliveries — and per the upstream recheck, the vocabulary flows
FROM wandr's six-consumer-unioned records (upstream's pointer events
are {x,y}-only today).

## Overlap audit

| Pair | Verdict |
|---|---|
| `surface.width/height` vs `canvas.width/height` | different objects — window geometry vs THIS buffer's drawable size (offscreen canvases have no surface at all) |
| `surface-events.configure` vs `frame-handler.on-resize` | the push/pull AXIS of one vocabulary; embedder routes exclusively — never both |
| `get-context()` here vs `wasi:canvas/embedding.get-context()` | fused vs general form of the same grant: `embedding.get-context()` ≡ `primary-surface.get-context()` + canvas connection, for reactor guests with exactly one surface; documented equivalence, both legitimate |
| `request-set-size` vs arbiter geometry | request/configure — the request is advisory BY CONTRACT, so no authority overlap with the window server |

## Final acceptance check

| Criterion | Status | Evidence |
|---|---|---|
| Architecturally clean | **PASS** | one socket, four producers; geometry/events/buffers separated; windowing authority stays with the embedder (request/configure); event vocabulary not duplicated |
| No overlapping functionality | **PASS** | audit above — fused forms are documented equivalences with un-fuse lanes, not parallel contracts |
| WASI-acceptable | **PASS (design level)** | shapes wasm-tools-validated; FIXES upstream's ambient-constructor violation; pollables only in the optional pull interface (R4: the reactor class never needs them); charter scope (graphics) ✓; interposition ✓ (a component can implement surface over a host's surface) |
| 100% consumable | **PASS by construction** | the consumers of THIS layer are the four producer classes + both guest classes — all four producers have a stated connection idiom; both event profiles are served; the reference UI libraries sit one layer up (on canvas + input-handlers) and need nothing from here directly |

## Status / sequencing

Design-only, deliberately unwired: wandr's host already PLAYS the
surface+socket roles internally (arbiter geometry + sf_media child
surfaces + the renderer), so nothing ships until one of three triggers:
an engine-class guest (wasi-gfx consumer), the wandr:video factoring,
or the upstream standards conversation. When triggered, implementation
is a re-skin of existing machinery — the same situation `scene` is in
with the WasiDrawable code.

## Wiring plan — the day upstream wasi-gfx stabilizes

What concretely changes, by layer (kept current so the trigger costs a
checklist, not a rediscovery):

**WIT — our packages need NO breaking change (by construction):**

| Package | Change | Class |
|---|---|---|
| wasi:canvas | the un-fused connection entry EXISTS: `wit-0.0.2/connection.wit` (validated, outside the canvas-host world) — wiring = add it to the world + host impl | ADDITIVE, drafted |
| wandr:video → `wasi:video-decoder` | the factored draft now EXISTS: `proposals/wasi-video-decoder/` (decoder + `connect(ctx)`; placement/visibility/z moved to the surface vocabulary; CVO `set-rotation` stays — a property of the frames). Fused wandr:video keeps shipping side-by-side (R3) | NEW greenfield package, validated |
| wasi:input-handlers | nothing — the push profile is delivery-complete; the pull profile is upstream's `surface-events` pollables consuming the SAME vocabulary | none |
| upstream wasi:surface/graphics-context | imported as published (plus our DESIGN.md change-set offered upstream); never authored by us | — |

**Host (wandr-host) — a re-skin of machinery already running:**

1. Bindgen modules for the published `wasi:surface` + `wasi:graphics-context`
   (imports, the input-handlers pattern).
2. `SurfaceRes` over the task-93 `sf_media` child SurfaceControl (android)
   — create-desc → child surface; `request-set-size` forwarded to the
   arbiter as an ADVISORY (request/configure); configure events fed from
   the arbiter's existing geometry pushes. Desktop: a child viewport of
   the winit window (the overlay pattern).
3. `ContextRes` binding one surface to one producer; `abstract-buffer`
   realization per producer: frame-buffer = CPU upload into the BBQ
   producer (the task-93 present path); canvas = a skia surface targeting
   the child surface (the renderer's Main-canvas machinery, re-targeted);
   video = MediaCodec output surface (what decode-to-surface already does
   internally — `connect` just names it).
4. Pull-profile pollables: per-surface event queues exposed via
   `wasi:io/poll` pollables on HostState (the convergence proposal's
   step-3 spike) — reactor guests never touch this path.
5. Loader: linker additions + probes, same shape as every package.

**Out of scope at wiring time** (already settled): window creation policy
stays with the arbiter; the reference UI libraries keep consuming
canvas + input-handlers and notice nothing.

## Named deferrals

| Deferral | Lane |
|---|---|
| Child-surface creation policy (who may create, z-order, roles) | embedder policy interface (wandr: the arbiter contract) — deliberately NOT in the standard surface, same reasoning as every compositor |
| atomic `on-configure` (size+scale) for the push profile | input-handlers 0.0.3, additive-interface lane |
| Surface hints (opaque, fullscreen, title) | additive methods (R2) when a consumer ships the need |
