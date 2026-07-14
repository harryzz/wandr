# App-side wandr code — RULES (BINDING, interim: "for now")

An OpenSwiftUI-on-wandr **app** carries ONLY these three wandr-specific seams:

1. **AUDIO** — the app's audio shim/bridge.
2. **STORE** — the app's persistence/store shim.
3. **STARTUP (wasm)** — the wasm entry seam: names the root scene and boots the app.

## NOTHING ELSE belongs in the app.

The following is GENERIC wandr↔OpenSwiftUI **runtime glue** and MUST NOT live in
app code. It is identical for every app; today it is duplicated in the app targets
only as interim scaffolding, and will be extracted to a shared runtime module:

- the `WandrDrawSink` conformer (`CGSink`) — the DisplayList → wasi:canvas bridge
- the reactor render loop + canvas acquire/render/present (`onFrame`)
- input / frame-pacing exports (`onResize`, `onPointer`, `nextFrameDelay`)
- ANY rendering-feature forwarding — clip, path fill, blur, 3D, gradients, …

## Rendering features are added ONCE, in the shared runtime — never per app.

If an app needs oval buttons (rounded corners), blur, a tilt, etc. it supplies
**nothing**: the shared runtime already implements the effect. A feature that
requires per-app code is a design bug in the runtime, not the app's job.

## Rationale

An app = its **views** + the **three seams** above. Anything a second app would
copy verbatim is runtime, not app.

## Status (interim)

The generic runtime currently lives, DUPLICATED, in:
- `Sources/T2iles/WandrReactor.swift`
- `Sources/OpenSwiftUIDemo/main.swift`

Extraction into a single shared runtime (a shared target here, and ultimately a
productized `OpenSwiftUIWandr` module in the OpenSwiftUI package that keeps
OpenSwiftUICore platform-independent) is PENDING. Until then, do not grow the
per-app copies with anything beyond Audio / Store / startup — add runtime
features in a way that will move cleanly into the shared runtime.
