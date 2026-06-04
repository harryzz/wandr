---
name: project_arbiter_sensors
description: wart-arbiter-sensors SensorService (task 77) + shared wart-hal-sensors crate — DONE+device-verified
metadata: 
  node_type: memory
  type: project
  originSessionId: a6ba002c-9c9c-4673-9e97-6c4e1c3eba6d
---

Task 77 — the arbiter's **SensorService**, built FIRST as the home for all sensor
consumers. DONE + device-verified (Pixel 2 XL, 2026-06-04). UNCOMMITTED.

**Three pieces:**
- `runtime/wart-hal-sensors/` — NEW shared crate: the ONE binder-touching owner of
  `android.frameworks.sensorservice.ISensorManager` (rsbinder, event-queue `Bn`
  callback). Neutral structs (`HalSensor`/`HalSample`). Used by BOTH the arbiter
  driver AND wart-host (`sensors_impl.rs` refactored to a thin WIT adapter; the 3
  sensorservice `.source()` lines removed from `wart-host/build.rs`). Standalone
  package (own `[workspace]`), AIDL vendored once under `wart-host/vendor`,
  codegen'd in its own build.rs (android-only). `ensure_process_state()` lazily
  inits rsbinder ProcessState + thread pool — the arbiter had NO binder before, so
  this was required (panic "ProcessState is not initialized" without it).
- `wart-arbiter-sensors` — pure module: enable-on-demand ref-count (battery), raw→
  semantic proximity with hysteresis DERIVED from HAL `max_range` (mid = max_range
  × 0.5, band × 0.1 — no hardcode; resolution is useless on binary prox sensors
  that report resolution==max_range). Verbs: `report-sensor <kind> <x>` (sim),
  `sensor-state`, `sensor-hold <kind> <on|off>` (manual HAL enable for testing).
- binary `sensor_driver.rs` thread (models `spawn_alarm_timer`): enumerate→seed
  Store descriptors, `Effect::SetSensor`→HAL enable/disable, poll→`bus_emit
  Event::SensorReading`. `Effect::SetSensor` arm in `execute_effects`.

**Consumer protocol (decoupling):** consumers emit `Event::SensorAcquire/Release`;
the sensors module ref-counts + drives `Effect::SetSensor`; readings come back as
`Event::SensorReading`→translated to `Event::ProximityChanged`. Modules never call
each other. Proof consumer: `wart-arbiter-power` acquires proximity on
`CommsActive`, logs on `ProximityChanged` ("would blank now").

**Core seam:** `SensorKind`, `Effect::SetSensor`, `Event::{SensorAcquire,
SensorRelease,SensorReading,ProximityChanged}`, `Store::SensorSlot`.

**Follow-ons (out of scope, each +1 reaction):** proximity-screen-off POLICY +
the host **panel-blank applier** (the real gap — `power`'s `doze 0` only keeps the
proc alive, no active blank op); migrate orientation off `report-orientation` onto
a live accel read via this service; light→auto-brightness; gesture sensors.

Device gotchas: Pixel 2 XL proximity is binary (handle 6, max_range≈5.0,
resolution==max_range); reads x=0 near / x=5 far; vendor HAL logs (`ASH:
ams_deviceSetConfig: Enabling/Disabling proximity`) confirm physical power-down.
Related: [[project_arbiter_audio]] (CommsActive producer), [[feedback_no_hardcoding]],
[[reference_rsbinder_version]].
