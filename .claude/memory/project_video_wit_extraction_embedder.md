---
name: project_video_wit_extraction_embedder
description: "NEXT-SESSION task: verify the wasi:video-codec extraction (codec-only BASICS, WASI-COMPLIANT = portable/capability-based/virtualizable/standards-aligned/n&s), re-express wandr:video as the EMBEDDER that IMPORTS it, minimal proof-app rewire, optional wasi-gfx connection sketch. GOAL = clean WIT proposals; apps are necessary-and-sufficient PROOF, NOT products; nothing ships."
metadata:
  node_type: memory
  type: project
  originSessionId: a14a1f7f-f5fb-44f9-a0e5-3879acecf911
  modified: 2026-08-06T06:24:57.901Z
---

# Video WIT extraction → wandr:video as embedder (fresh-session task)

## ‼️ THE GOAL (read first — I kept getting this wrong)
The deliverable is **clean WIT proposals**. **Nothing ships.** The apps
(Signal, jellyfin, video.player, media-engine) are **PROOF harnesses that a WIT
is necessary-and-sufficient** — they get rewired freely; "don't break the app /
migration cost / coexistence" is NOT a constraint. Do not treat any contract as
a product to protect.

## THE NAMESPACE RULE (the user's, verbatim intent)
- **`wasi:*`** = BASIC primitives a guest **cannot do itself / cannot do
  without** (universal, proposable to WASI).
- **`wandr:*`** = **our stack's own decisions** (how wandr chooses to do it).

## THE ARCHITECTURE = import LAYERING (not a fork, not two copies)
1. **`wandr:video` STAYS** — the proven fused contract, and the **embedder** the
   apps import.
2. **Extract the BASICS out of it** into `wasi:video-*`:
   `wasi:video-codec` = `frame` + `decoder` + `encoder` (HW codec a guest can't
   do itself), and NOTHING else. (`wasi:camera` = capture source, same.)
3. **`wandr:video` then `use`s/imports the extracted wasi wits** and keeps only
   the wandr-stack parts on top: host-fill decode-to-surface, child-surface
   z-layers, `present(frame, at-ns)`, camera-capture glue, PiP self-view.
4. **PROOF = it imports cleanly.** If `wasi:video-codec` drops into `wandr:video`
   with no changes → it's a self-contained, valid WASI proposal. If the import
   forces a change → that's the feedback, fix the extraction, repeat. When
   stable, `wandr:video` is just the embedder wrapping proposable wasi basics.
   → codec change touches `wasi:video-codec` (wandr:video inherits via import);
   presentation change touches `wandr:video` only. One place each.

## ‼️ WASI COMPLIANCE — acceptance criterion for every `wasi:*` extraction
An extracted `wasi:video-codec` / `wasi:camera` is only valid if it complies with
WASI design requirements (this is WHAT makes it proposable, alongside n&s):
- **Portability** — implementable on ANY host/OS with NO OS-specific types, names,
  or assumptions: Android HW MediaCodec, desktop GStreamer/libvpx, a pure-SW
  fallback must ALL satisfy it. The portability mechanism is capability negotiation
  — `probe(config) -> support` (= WebCodecs isConfigSupported) + `acceleration`
  preference with graceful SW fallback. A minimal host advertises `unsupported` and
  the guest copes. Anything a specific backend needs but others can't provide →
  it's `wandr:`, not `wasi:`.
- **Capability-based, NO ambient authority** — access only via explicitly-granted
  handles (resources: `decoder`/`encoder`/`frame`); no global/implicit device or
  hardware reach. (WASI's core security principle.)
- **Virtualizable** — implementable by ANOTHER Wasm component, not only a native
  host (a guest could stand in for the host).
- **WIT conventions** — kebab-case; versioned package `@x.y.z`; interfaces + a
  guest/host `world`; `resource` for handles, `result<_, error>` for fallible ops,
  `enum`/`variant`/`record` used idiomatically; discriminants ABI-stable (APPEND
  only).
- **Standards alignment** — mirror the established web standard where one exists
  (WebCodecs `VideoDecoder`/`Encoder`/`VideoFrame` for the codec; W3C Media Capture
  for camera) so the proposal is recognizable and defensible, not invented.
- **Necessary-and-sufficient** — every verb has a proving consumer; nothing
  host-specific, speculative, or unprovable leaks in.
Verifying this compliance (per interface, with evidence) is PART of task A — deploy
agents to check against the WASI design principles + the W3C spec, as done for the
audio-codec/camera audits earlier.

## VERIFIED FACTS (3 source-grounded agents, this session — do NOT re-derive)
- **Upstream `wasi-gfx:surface@0.2.0` is GUEST-FILLS ONLY.** A pairing context is
  `resource context { constructor(borrow<surface>); get-current-buffer()->buffer;
  present(); }` — the GUEST acquires the buffer, writes pixels, calls present().
  NO host-fill path, NO `attach`/producer verb; `present()` is marked
  `// TODO: consider if needed`; issue #55 (WebAssembly/wasi-webgpu) still OPEN;
  `wasi:graphics-context` DEPRECATED/removed. (github.com/wasi-gfx/wasi-gfx
  packages/surface/wit/.) See [[reference_wasi_gfx_ecosystem_relation]].
- **Host-fill decode-to-surface (wandr's zero-copy video) is NOT expressible on
  upstream surface** → it stays `wandr:` (embedder), never `wasi:`. This is
  CORRECT by the namespace rule (compositing decoded frames the wandr way IS a
  stack decision), not a fallback.
- **Canvas is guest-PUMP** (`get-current-buffer`/`present` each frame) → genuinely
  matches upstream → a `surface-canvas` pairing is legit for CANVAS only.
- **Video/camera are host-fill / claim-not-pump** → they use NEITHER upstream
  verb → NO `surface-*` pairing for them (it would be a wandr extension mislabeled
  as "wasi-gfx-aligned"). The uniform "two-form template for all producers" I
  recorded earlier was a FALSE SYMMETRY — refuted.
- In-tree consumer map: **Signal** (`engine/src/call.rs`) drives encoder +
  host-camera + PiP + RTP decoder (RTP `submit`, NOT `submit-timed`); **video.player**
  + **media-engine** drive decoder playback (`open-accelerated`/`submit-timed`/
  `next-decoded`/`presented-rect`); **jellyfin** has NO direct video verbs — it
  delegates to `wandr-media-engine`.

## THE TASKS (fresh session)
**A. VERIFY the current extraction is the right codec-only BASIC set + WASI-COMPLIANT**
(see the WASI COMPLIANCE criterion above — portability / capability-based / virtualizable /
WIT conventions / standards-aligned / n&s; verify per-interface with agents + the W3C spec).
- `contracts/proposals/wasi-video-codec/wit/video-codec.wit` currently has
  `connect(ctx: borrow<context>)` + `use wasi:graphics-context@0.0.2` + surface
  language. **STRIP all surface from it** — extracting surface into the codec was
  the mistake; surface = embedder = wandr. Leave: types (codec/accel/support/
  codec-error/encoded-chunk/decoder-config/encoder-config), `frame`, decoder
  (probe/open/submit/next-decoded/flush/reset — surface-agnostic), encoder
  (probe/open/encode(frame)/next-chunk/request-keyframe/set-bitrate). `wasi:eme`
  keys dep is fine (DRM, our draft). Template: `wasi:audio-codec` is ALREADY the
  clean pure-codec shape (no surface) — mirror it.
- OPEN QUESTION to resolve: the **frame / decoded-frame / present boundary.**
  WebCodecs has no `present`. Likely: `wasi:video-codec` decoder emits an opaque
  `frame` via `next-decoded` (NO present); `present(frame, at-ns)` is a
  `wandr:video` (embedder) verb. Confirm where `decoded-frame` + `present` +
  `set-rotation` + `ready` land (codec vs embedder).
- Same strip for `wasi:camera` (drop `connect-preview(ctx)`; camera = source that
  PRODUCES `frame`).

**B. PROPOSE the embedding — re-express `wandr:video` as importer + embedder.**
- `wandr:video` `use`s `wasi:video-codec.{frame, decoder, encoder, encoded-chunk,
  codec, …}` and adds the wandr-stack interfaces: a surface/present layer
  (`present(frame, at-ns)`, `set-rect`, `set-visible`, `z-layer`, host-assigned
  CHILD surface — the current SurfaceView model in `contracts/wit/video.wit`),
  camera-capture glue (camera→encoder, using `wasi:camera`), and PiP self-view.
- Resolve: AUTO-present (RTP calls, guest never pulls) needs the embedder to bind
  the decoder's output to a host surface — a binding verb in `wandr:video`
  (e.g. `surface.attach(borrow<video-decoder>)`), NOT in `wasi:video-codec`.

**C. MINIMAL wiring refactor (proof).** Rewire the proof apps to the layered
`wandr:video` world (which now bundles the wasi codec + wandr embedder). This
re-proves necessary-and-sufficient; it is not a cost. Apps: Signal call.rs,
video.player, media-engine, (jellyfin via engine).

**D. OPTIONAL / exploratory — sketch `connection` to wasi-gfx.**
- Legit ONLY for the guest-pump case: a `surface-canvas` pairing for
  `wasi:canvas` against `wasi-gfx:surface@0.2.0` (`context::constructor(surface)`
  + `get-current-buffer` + `present`) — canvas genuinely matches upstream.
- For VIDEO/CAMERA (host-fill): a wasi-gfx `connection` does NOT align (upstream
  is guest-fills). If sketched at all, frame it as a FORWARD-LOOKING proposal —
  "what a host-fill / decode-to-surface extension to wasi-gfx:surface would need"
  (issue #55 is open) — NOT a `wandr:video` dependency.

## CONSTRAINTS
- **Every `wit/` edit needs explicit approval (CLAUDE.md rule #4)** — present
  sketches for review before writing. See [[feedback_wit_changes_need_approval]].
- The drafts are UNWIRED (no host impl, no guest imports the drafts) → editing
  them breaks no consumer; `wandr:video` is the only wired video contract today.
- Diagnostics already split out: `wandr:video-diag` / `wandr:audio-diag`.

## FILES
- `contracts/wit/video.wit` — wandr:video (fused embedder, to re-express as importer)
- `contracts/proposals/wasi-video-codec/wit/video-codec.wit` — the extraction (STRIP surface)
- `contracts/proposals/wasi-camera/wit/camera.wit` — strip connect-preview
- `contracts/proposals/wasi-canvas/wit/{embedding.wit,connection.wit}` — the guest-pump template (embedding wired; connection on the dead graphics-context)
- `contracts/proposals/wasi-audio-codec/wit/audio-codec.wit` — the clean pure-codec TEMPLATE to mirror
- Host: `runtime/wandr-host/src/{video_host_impl.rs,video.rs,wasi_canvas_impl.rs}`
- Guests: `apps/user/wandr.signal/engine/src/call.rs`, `apps/user/wandr.video.player`, `crates/wandr-media-engine`, `apps/user/wandr.jellyfin`

Related: [[reference_wasi_gfx_ecosystem_relation]] · [[feedback_wit_changes_need_approval]]
· [[project_wandr_video_host]] · [[project_wandr_call_video_track]] · [[project_task115_wasip3_async]].
