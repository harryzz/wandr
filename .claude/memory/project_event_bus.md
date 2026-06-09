---
name: project_event_bus
description: wandr:events — generic host↔guest pub/sub event bus (task 90 M1) — DONE + device-verified
metadata: 
  node_type: memory
  type: project
  originSessionId: a2edab94-9d77-4289-807e-6fabf67af25c
---

`wandr:events@0.1.0` (`wit/events.wit`) — a generic in-process publish/subscribe event bus so new event types cost ZERO new WIT/host wiring (just a topic string), replacing the per-feature `*-handler` export pattern (alarm/notify/connectivity each had their own bindgen world + InboundEvent variant + drain arm). ✅ M1 DONE + device-verified 2026-06-09 (Pixel 2 XL --no-art), first consumer = connectivity (`net.status`).

**Why not exact `wasi:messaging`:** researched the WASI proposal first (it's the standard, shape matches: `producer.send` + `incoming-handler.handle`). But its `message`/`client` are **resources** + connection/broker-oriented (Kafka/NATS, Phase-4 needs 2 broker impls) + **no turnkey host crate** (wasmtime ships wasi-http/config/io, NOT messaging). Resources also risk the guest wit-parser (the project already redeclares trimmed WIT). So: built `wandr:events` with the proposal's VOCABULARY (`types.message{topic,content-type,data}`, `producer.publish`, `incoming-handler.handle`) using a plain **record** instead of resources — forward-compatible (swap record→resource + add client to migrate), no resource tax. User's driver: "make things standard + reusable". (Aborted a half-built exact-`wasi:messaging` attempt — the resource Host-trait wrangling was the tax I'd flagged; bindgen resource `with`-key is `"pkg:ns/iface.resource"` with a DOT, e.g. `wasi:filesystem/types.descriptor`.)

**Architecture (mirrors the alarm push-delivery path):**
- WIT: `interface types{record message}`, `interface producer{publish}`, `interface incoming-handler{handle}`; worlds `events-host`(import producer), `events-incoming`(export incoming-handler), `events-client`(both). Receive-only guest = just `export wandr:events/incoming-handler`.
- Broker = `wandr-arbiter-events` (`runtime/wandr-arbiter/wandr-arbiter-events`): `HashMap<topic,Vec<pid>>` subscribers + `HashMap<topic,String>` retained; verbs `evt-subscribe <pid> <topic>` / `evt-unsubscribe` / `evt-publish <topic> <b64payload>`; fans `event <topic> <b64>` via `Effect::HostLine`; retained value delivered on subscribe (MQTT-retained); drops dead pids on `Event::SurfaceRemoved`. Registered 1-line in build_registry; CLI verbs for testing. 3 unit tests.
- Host: `events_host_impl.rs` — `producer::Host::publish` → arbiter `evt-publish` (+ base64 helpers, payload is opaque b64 on the line-framed socket); empty `types::Host` impl needed for bindgen. `EventsHost::add_to_linker` (both app_loader paths). `events-incoming` export binding = `Option<EventsIncoming>` on InstantiatedApp; `ime_inbound` parses `event <topic> <b64>` → `InboundEvent::Event`; standalone drain constructs a `Message` + `call_handle`.
- **Subscription = host-config, NOT a WIT call** (the wasi:messaging delivery model): guest declares topics in `package.toml [events] subscribe = ["net.status"]`; `app_loader::event_subscriptions()` reads it; the host sends `evt-subscribe` — **deferred until after the control socket is bound** (`spawn_listener`), else the retained-on-subscribe delivery races a not-yet-listening socket (the first-launch bug I hit + fixed).
- Connectivity publisher: `wandr-net` daemon `report()` sends `evt-publish net.status <b64(online wifi <ssid> <ip> | offline)>` alongside the existing `report-net-state` (NetModule stays for status-bar/CLI). Payload = same wire string the guest decodes.

**Guest gotcha (device-debugged):** dioxus-canvas only re-runs the component when the renderer is `mark_dirty`'d. The `incoming-handler.handle` export runs OUTSIDE the render path (can't touch `r`), so it must set a thread-local DIRTY flag that `pre_frame` consumes → `r.mark_dirty()` (mirrors Signal's `pump`→`mark_dirty`). Without it `handle` updated state but the UI never repainted (host logs showed `handle` dispatched 4× while UI stuck at "0 events"). The host forces a frame on event delivery (`dirty=true` in the drain), but that re-PAINTS, doesn't re-RUN the VDom.

**Test guest** `apps/user/wandr.connectivity.test` (dioxus, "Net Monitor"): subscribes `net.status`, renders Online/Offline + transport/ssid/ip. Device-verified: Online↔Offline flips live on publish (no polling); event counter increments. Deploy = per-app `--install` + `run-hybrid-stack --wandr-only` (see [[reference_wandr_apps_root_install]]). Related: [[project_artless_network]] (the connectivity subsystem this rides on), task 90 (`tasks/90-connectivity-wit-implementation.md`).

**Follow-ups:** migrate alarm-fired + notification-clicked onto the bus (delete their bespoke `*-events`); task 90 M2–M4 (wifi mgmt) still use the typed `wandr:connectivity/wifi`. Minor: stale subscriber pid until SurfaceRemoved fires (harmless failed deliver).
