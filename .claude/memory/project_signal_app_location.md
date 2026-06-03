---
name: project-signal-app-location
description: Where the Signal app source lives + how to build/deploy it
metadata: 
  node_type: memory
  type: project
  originSessionId: 81538868-ab9d-48a4-8de3-a56739b11c3e
---

**ROLE: war.signal is the runtime's capability-PROOF demo, not the product.** It
was chosen because it brutally exercises EVERY subsystem at once — Compose/GPU
display, audio capture+playback+Opus+SRTP+WebRTC media, background receipt while
not foregrounded, the alarm/scheduler keep-alive, networking, crypto,
notifications, keyguard. A real client that interops with Google's libwebrtc can't
cut corners, so "Signal works end-to-end" is the demonstration that wart can
replace ART. Corollary for diagnosis: when something "in Signal" breaks, look in
the RUNTIME LAYER first — Signal is usually just the lens exercising a gap, not
the bug's home. (User framing, 2026-06-04.) [[feedback_read_source_first]]

The Signal app (text-only Signal client, tasks 67/69/70) was promoted out of
`repros/` into **`apps/user/war.signal/`** on 2026-05-31:

- `apps/user/war.signal/engine/` — the wasm32-wasip2 engine component exporting
  `wart:signal/chat` (was `repros/signal-engine`). Path deps are 4-up
  (`../../../../external/...`).
- `apps/user/war.signal/ui/` — the dioxus-canvas guest importing `chat` (was
  `repros/signal-ui`).
- `apps/user/war.signal/package.toml` — the warpkg manifest (was already here).
- `apps/user/war.signal/build.sh` — builds both, `wac plug`s ui◁engine, assembles
  `build/war.signal.warpkg`; `./build.sh --deploy` also installs + relaunches on
  device (keeps `/state`). Replaces the old manual wac-plug-into-`/tmp` dance.

Still in `repros/` (correctly — genuine spikes / drivers, NOT the app):
`signal-phase0` (Phase 0 de-risk), `signal-link` (Phase 1 CLI),
`signal-engine-smoke` (Rust CLI verification driver, `wac plug`'d onto the engine).

See [[project_signal_client_architecture]], [[project_wart_step_executor]].
