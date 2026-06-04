# Task 77 — `wart-arbiter-sensors` (the arbiter's SensorService)

> Status: 🔲 scoped, not started — fresh session.
>
> Establish the **sensors responsibility crate now**, before any sensor
> consumer (proximity-screen-off during calls, accel→orientation,
> light→auto-brightness, shake gestures) is built — so each consumer plugs in
> by **reacting to a sensor event**, not by patching ad-hoc HAL access that
> we'd later have to refactor into a service. This is the SensorService
> analog: a distinct Android system service ⇒ its own arbiter module, per the
> design law in `docs/visual-sizing-design-patterns.md` ("a core + responsibility
> crates… +1 crate, +1 line; modules never call each other — they react to events").

## Why now (not a power-module patch)

A proximity-screen-off-during-call feature *could* be hacked straight into
`wart-arbiter-power`. But the moment a second sensor consumer appears
(orientation from the accelerometer — today still a `report-orientation` push,
not a live read; auto-brightness from the light sensor; etc.) we'd be reading
the HAL from multiple places with no enable/disable arbitration (battery) and no
single owner. Build the owner first. After this lands, proximity-screen-off is a
**separate, tiny follow-on** in `wart-arbiter-power` that just reacts to
`Event::ProximityChanged` (see "Out of scope").

## Current state (verified)

- The arbiter reads **no sensor HAL**. Orientation enters via the
  `report-orientation <raw>` verb (`wart-arbiter-wm`), which converts it and
  emits `Event::OrientationChanged`. So there is no HAL polling, no battery
  arbitration, no sensor lifecycle today.
- The **host** already reads sensors for *guests* via the `skiko-gfx`
  `sensors` WIT interface → `runtime/wart-host/src/sensors_impl.rs` →
  `android.frameworks.sensorservice.ISensorManager` (stable AIDL, Android 11+).
  **That is the reference for the AIDL + event-channel mechanism** — but it's the
  *guest-facing* path; the arbiter needs its **own** rsbinder access (the arbiter
  is the persistent system coordinator; host children are per-app + ephemeral).
- Module pattern (core, `runtime/wart-arbiter/wart-arbiter-core/src/lib.rs`):
  modules are **pure** — `verbs()`, `on_command()`, `on_event()`; they `emit`
  `Event`s + `request` `Effect`s. The **binary** (`wart-arbiter-bin`) owns IO and
  runs hardware threads — the **screen poller** `bus_emit`s `Event::ScreenState`
  and the **alarm timer** emits `Event::AlarmTick`. The sensor HAL driver is the
  same shape.

## Design

```
   ISensorManager (HAL, rsbinder)
        │  poll / event channel        ┌──────────────────────────────┐
        ▼                              │  wart-arbiter-sensors (pure)  │
   binary sensor-driver thread  ──bus_emit Event::SensorReading──▶     │
   (wart-arbiter-bin)           ◀──Effect::SetSensor{kind,on,rate}── policy:
        │ enable/disable per Effect     │  - ref-count enable-on-demand │
        ▼                              │  - raw → semantic events      │
   (only-enabled sensors draw power)   │  - Store: live sensor state   │
                                       └──────────────┬───────────────┘
                                                      │ emit
                                       Event::ProximityChanged{near}, …
                                                      │
                              consumers REACT (never touch the HAL):
                              power → screen-off (follow-on), wm → orientation, …
```

- **`wart-arbiter-sensors` module (pure policy):**
  - **Enable-on-demand arbitration** — ref-count consumers per sensor `kind`;
    `request(Effect::SetSensor{kind, on:true})` on the first consumer,
    `on:false` when the last drops. This is the battery contract and the whole
    reason the service exists.
  - **Raw → semantic translation** — turn `Event::SensorReading{kind, sample}`
    into consumer-friendly events with hysteresis/debounce, e.g.
    `Event::ProximityChanged{near}` (proximity is a scalar in `sample.x`; "near"
    = below the sensor's `max-range`/threshold, with debounce so a flicker
    doesn't toggle the screen).
  - **Store state** — last known value per enabled sensor (so a new consumer or
    `sensor-state` verb reads current state without waiting for the next sample).
  - **Verbs** (testing without the HAL, mirroring `report-orientation`):
    `sensor-state`, and a `report-sensor <kind> <x> [y z]` sim verb that injects
    a reading so the policy + consumers can be exercised on a desktop / before the
    HAL path is wired.
- **Binary sensor-driver thread (`wart-arbiter-bin`):** the only place that
  touches the HAL. Model on `spawn_alarm_timer` / the screen poller. Applies
  `Effect::SetSensor` (enable/disable a sensor on `ISensorManager` at the
  requested rate), reads samples off the sensor event channel, and `bus_emit`s
  `Event::SensorReading{kind, sample}`. Mirror `sensors_impl.rs` for the AIDL +
  channel.
- **Core additions (`+1` each, non-invasive):**
  `Effect::SetSensor { kind: SensorKind, on: bool, rate_hz: u32 }`,
  `Event::SensorReading { kind, x, y, z, ts_ns }`, and the semantic
  `Event::ProximityChanged { near: bool }` (add more semantic events as
  consumers land). A small `SensorKind` enum (reuse the WIT `kind` set:
  proximity, accelerometer, light, …).

## Steps

1. **Core seam.** Add `SensorKind`, `Effect::SetSensor`, `Event::SensorReading`,
   `Event::ProximityChanged` to `wart-arbiter-core`. (No behaviour yet — just the
   vocabulary so the rest is `+1 reaction`.)
2. **Scaffold the crate.** `runtime/wart-arbiter/wart-arbiter-sensors`
   (Cargo.toml dep on `-core` + `log`, like `wart-arbiter-audio`); `SensorsModule`
   impl of `ArbiterModule` with `verbs() = ["sensor-state","report-sensor"]`,
   ref-count map, raw→semantic translation, Store writes. Register it in
   `wart-arbiter-bin/src/main.rs` (`reg.register(Box::new(SensorsModule::new()))`).
   **Drive it end-to-end with the `report-sensor` SIM verb first** (no HAL) — prove
   proximity near/far events + debounce on the bus before touching binder.
3. **Binary HAL driver.** Wire `ISensorManager` via rsbinder (vendor the AIDL if
   not already shared with the host; reference `sensors_impl.rs` for the
   enumerate + event-channel mechanism). Spawn the driver thread; apply
   `Effect::SetSensor`; `bus_emit` real `Event::SensorReading`. Device-verify
   proximity readings flow.
4. **First real consumer (proof the seam works).** Wire `wart-arbiter-power` to
   ref-count proximity **while a call is active** (`Event::CommsActive` →
   acquire; end → release) so the sensor is only on during calls, and have it
   react to `Event::ProximityChanged`. Stop at *logging* "would blank now" — the
   actual screen-blank applier + the policy is the **follow-on** task below, so
   this task closes with the SERVICE proven, not the screen behaviour.

## Out of scope (deliberately — these are CONSUMERS, follow-on tasks)

- **Proximity-screen-off-during-call policy** + the **host screen-blank /
  panel-power applier** — a separate `wart-arbiter-power` task that *reacts to*
  `Event::ProximityChanged`. The host still lacks an active panel-blank op
  (`power`'s `doze 0` only keeps the process alive with the screen off); that
  applier is the real build-gap there.
- **Migrating orientation** off the `report-orientation` push onto a live
  accelerometer read through this service — a clean follow-on once the service
  exists (keep `report-orientation` working meanwhile).
- Per-app **sensor focus / direct-channel** semantics, light→auto-brightness,
  gesture sensors — future consumers, each `+1 reaction`.

## Open questions / risks

- **rsbinder `ISensorManager` event delivery on the Pixel 2 XL** — does the
  vendor HAL give a usable poll/`getSensorList`+queue path, or does it need the
  direct-channel (shared-mem) route? `sensors_impl.rs` already solved this for the
  host; confirm the same works from the arbiter process (sepolicy/AVC for the
  arbiter reading sensorservice — `rsbinder-triage` if denied).
- **Battery** — enable-on-demand is mandatory; a left-on proximity/accel drains.
  The ref-count + `SetSensor{on:false}` on last-release is the contract; verify the
  sensor actually powers down (dumpsys / power draw) when no consumer holds it.
- **Debounce thresholds** — proximity "near" hysteresis so a hand-wave doesn't
  flap; derive from the sensor's reported `max-range`/`resolution`, don't hardcode
  ([[feedback_no_hardcoding]]).
- Whether to fold orientation now (one fewer push path) or after — lean *after*,
  to keep this task to the service + one consumer proof.

## Where it lands in code

- `runtime/wart-arbiter/wart-arbiter-core/src/lib.rs` — `SensorKind`,
  `Effect::SetSensor`, `Event::SensorReading` + `Event::ProximityChanged`.
- `runtime/wart-arbiter/wart-arbiter-sensors/` — new crate (the module).
- `runtime/wart-arbiter/wart-arbiter-bin/src/main.rs` — register the module +
  the sensor-driver thread + `Effect::SetSensor` in `execute_effects`.
- Reference (don't duplicate the policy): `runtime/wart-host/src/sensors_impl.rs`
  (AIDL + event channel), `wart-arbiter-audio` (`Event::CommsActive` producer),
  `wart-arbiter-power` (the first consumer + the doze/screen precedent).

## Verification

- Desktop/unit: `report-sensor proximity 0` / `proximity 5` → module emits
  `ProximityChanged{near:true/false}` with debounce; ref-count enable/disable
  emits the right `SetSensor` effects.
- Device: with the HAL driver wired, cover/uncover the proximity sensor →
  `ProximityChanged` in the arbiter log; during a Signal call the sensor is
  enabled (CommsActive) and disabled on hangup (verify it powers down).
- The follow-on power task then turns `ProximityChanged` into an actual
  screen-off — out of scope here.
