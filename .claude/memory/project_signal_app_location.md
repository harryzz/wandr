---
name: project-signal-app-location
description: Where the Signal app source lives + how to build/deploy it
metadata: 
  node_type: memory
  type: project
  originSessionId: 81538868-ab9d-48a4-8de3-a56739b11c3e
---

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
