---
name: feedback_openswiftui_app_only_audio_store_startup
description: RULE — an OpenSwiftUI-on-wandr app carries ONLY Audio, Store, and wasm startup; all rendering/reactor/sink glue is shared runtime, never per-app
metadata:
  node_type: memory
  type: feedback
  originSessionId: 60b1802d-eb7e-41f1-b233-3fecc364fe2d
---

BINDING RULE (interim, "for now"): an OpenSwiftUI-on-wandr **app** contains ONLY three
wandr-specific seams — **AUDIO** (audio shim), **STORE** (persistence shim), and **STARTUP**
(the wasm entry that names the root scene + boots). NOTHING ELSE.

**Why:** everything else is GENERIC wandr↔OpenSwiftUI runtime glue, identical for every app —
the `WandrDrawSink` conformer (`CGSink`, the DisplayList→wasi:canvas bridge), the reactor render
loop / canvas acquire→render→present (`onFrame`), input+frame-pacing exports (`onResize`,
`onPointer`, `nextFrameDelay`), and ANY rendering-feature forwarding (clip, path fill, blur, 3D,
gradients). It is duplicated today ONLY as interim scaffolding (verbatim in
`repros/swift-canvas-spike/Sources/T2iles/WandrReactor.swift` AND
`Sources/OpenSwiftUIDemo/main.swift` — proof of the smell) and belongs in ONE shared runtime.

**How to apply:** rendering features (rounded/oval buttons, blur, tilt, …) are added ONCE in the
shared runtime — NEVER per app. If a feature needs per-app code, that's a runtime design bug, not
the app's job. Do not grow the per-app reactor copies beyond Audio/Store/startup. The real
clip/fill logic I added went correctly into the SHARED `WandrCG` module (`CGContext.clip(svgPath:)`
/`fill(svgPath:)`); only the thin `CGSink` forwarders sit in the app, and those move out when the
runtime is extracted. Extraction pending: a shared target in the spike now, and ultimately a
productized `OpenSwiftUIWandr` module in the OpenSwiftUI package (keeping OpenSwiftUICore
platform-independent — it must not import wasi:canvas). Rule file:
`repros/swift-canvas-spike/Sources/T2iles/RULES.md`. Related: [[feedback_clean_library_usage]],
[[reference_swift_openswiftui_wandr]], [[reference_openswiftui_gestures_offapple]].
