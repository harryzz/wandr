---
name: project_openswiftui_structural_cleanup
description: OpenSwiftUI-on-wandr next-session tasks — normalize package structure (CSwiftSpike→CWASICanvas leaf, OpenCoreGraphics CGContext target, wandr-runtime out of app), THEN frosted backdrop blur
metadata:
  node_type: memory
  type: project
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

Next session on the eleev 2048 / OpenSwiftUI-on-wandr port: do the STRUCTURAL cleanup before the
frosted **backdrop blur** (the last missing modal effect). Ordered task doc:
`swift/OpenSwiftUIProject/NEXT-SESSION-TASKS.md` (rationale in `swift/OpenSwiftUIProject/COMPONENTS-AND-BUILD.md`).

**Task 0 (priority, user-facing) = gesture/interaction BUGS** (added end of session): swipe registers
above the board; board swipe intermittently freezes (recovers after tap-tile-then-swipe); modal
buttons + menu items don't accept clicks (no Settings/About); game-over dialog isn't modal (clicks
reach the invisible hamburger behind → phantom "new game"). Root theme: hit-testing uses a flat
layout `hitFrame` that ignores render transforms (`.offset`, modal positioning) + modals don't block
background input. Likely one fix (route offset/modal subtrees through the responder tree's
transform-aware `containsGlobalPoints` instead of flat `hitFrame`) resolves most. READ the hit-test
path end-to-end first — this subsystem burned days before. Full detail in NEXT-SESSION-TASKS.md §0 +
HANDOFF-eleev-openswiftui.md "REMAINING PROBLEMS" + [[reference_openswiftui_gestures_offapple]].

Structural order (each unblocks the next):
1. **Split `CSwiftSpike` → standalone leaf `CWASICanvas`** (wasi:canvas draw bindings only; leave
   input/export trampolines with the runtime) — breaks the package cycle that blocks anything above
   the app from importing the bindings.
2. **Normalize OpenCoreGraphics** — add a SEPARATE `CGContext` target in the OCG package (deps:
   OCG-geometry + CWASICanvas); NOT in `OpenCoreGraphics`/`OpenCoreGraphicsShims` (OpenSwiftUI
   imports those 48× — would re-break the build, the whole reason OCG is pinned at geometry-only
   `050239b`). Retire vendored `WandrCG` + the dormant `harryzz/OpenCoreGraphics@wasm32-wasip1`.
3. **`wandr-runtime` shared product** (OpenSwiftUI + CWASICanvas): reactor + `@_cdecl` exports +
   `CGSink` + a `runWandrApp` runner beside the framework's `runStdoutApp`. App collapses to one
   product dep + `import OpenSwiftUI`; carries only Audio/Store/startup ([[reference_swift_openswiftui_wandr]],
   `Sources/T2iles/RULES.md`).
4. **Finish structure normalize** (apple-compat already extracted to `swift/apple-compat`).
5. **THEN frosted backdrop blur** (`.filter(.blur)`): the wasi:canvas contract has NO general
   layer/backdrop-blur verb (only per-paint `mask-blur`) → add a WIT verb (blur on `save-layer` or
   `set-backdrop-blur` on the scene layer) + host Skia impl; shared-WIT change
   ([[feedback_shared_wit_rebuild_all_consumers]]).

This session already LANDED (committed to `harryzz/OpenSwiftUI@wasm32-wasip1` + wandr
`openswiftui-eleev-2048`): clip/fill (oval buttons + rounded tiles), 3D tilt (rotation3DEffect via
canvas CTM), drop shadow (`.filter(.shadow)` via clip silhouette). POLISH TODO (anytime): shadow
contrast (`sigma=radius` tuning), tilt fidelity. Also this session: `ComputeStubs`→Compute
`Graph::print_cycle`, Apple shims → `swift/apple-compat`, submodule `origin` remotes repointed to
harryzz (Compute+OAG), OAG branch fixed to `main`. Related: [[feedback_openswiftui_app_only_audio_store_startup]],
[[reference_openswiftui_gestures_offapple]].
