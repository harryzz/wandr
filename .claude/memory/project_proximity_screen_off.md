---
name: project_proximity_screen_off
description: Task 78 proximity screen-off during calls (SurfaceFlinger setPowerMode applier) — DONE+device-verified
metadata: 
  node_type: memory
  type: project
  originSessionId: a6ba002c-9c9c-4673-9e97-6c4e1c3eba6d
---

Task 78 — the task-77 follow-on: proximity sensor blanks the panel during a call.
DONE + device-verified (Pixel 2 XL, 2026-06-04). The applier task 77 deferred.

**Mechanism: SurfaceFlinger `setPowerMode`** (proper HWC panel-off, not backlight).
Reached over rsbinder via a new shared crate `runtime/wart-hal-display/`
(mirrors [[project_arbiter_sensors]]'s wart-hal-sensors). De-risked on-device
BEFORE building:
- `SurfaceFlingerAIDL` (`android.gui.ISurfaceComposer`) transaction order matches
  the full upstream extract: getPhysicalDisplayIds=6, getPhysicalDisplayToken=7,
  **setPowerMode=8**. The pre-existing host `build.rs` 4-method ISurfaceComposer
  stub was MIS-TUNED (its "getPhysicalDisplayIds at code 4" actually hit
  createVirtualDisplay → NPE, misread as "round-trip OK").
- Root permission PROVEN: setPowerMode shares getPhysicalDisplayToken's exact gate
  (`checkAccessPermission`→`callingThreadHasUnscopedSurfaceFlingerAccess`), and
  `service call SurfaceFlingerAIDL 7 i64 0` returns the display token as root.
- `hal::PowerMode`: OFF=0, ON=2. Primary display id=0 (derive from
  getPhysicalDisplayIds, don't hardcode).

**Full ISurfaceComposer codegen now works** (rsbinder-aidl 0.9.0, was blocked at
0.7.0): the only fixes are (1) strip `0f`/`-1f` float-literal suffix in 4
parcelables (DisplayBrightness/DisplayMode/CaptureArgs/TrustedPresentationThresholds)
— done in OUT_DIR copies so no vendored file is mutated; (2) the ~9 native-backed
`rust_type "gui_aidl_types_rs::X"` types resolve to a NEW shim crate
`runtime/gui-aidl-types-rs/` (crate_name `gui_aidl_types_rs`, zero-sized stub
parcelables via rsbinder `impl_serialize/deserialize_for_parcelable!`,
`unimplemented!()` bodies — AOSP's own gui_aidl_types_rs is likewise stubs). None
of those types are on the setPowerMode path.

**Arbiter wiring:** core `Effect::SetDisplayPower{on}`; `wart-arbiter-power` tracks
`blanked`, blanks only while a call is active (`!comms.is_empty()`), toggles on the
debounced ProximityChanged transition; **3 fail-safes** force the panel back on
(far reading, CommsActive{false} with no calls left, SurfaceRemoved of last call) —
a stuck-off panel must never happen. `execute_effects` arm →
`wart_hal_display::set_display_power`.

**Device test path (no real call needed):** `wart-arbiter audio-call-start <pid>`
emits CommsActive → proximity auto-enabled (task-77 wiring) → cover sensor → panel
OFF → uncover → ON → `audio-call-end <pid>` while covered → panel ON (fail-safe) +
sensor disabled. All verified; `setPowerMode applied=true`.

**Task 79 (touch suppression, DONE+device-verified)** closed the cheek-touch
follow-on: host `input.rs` TOUCH_SUPPRESSED atomic gates `dispatch_pointer*` (the
single touch choke point); `ime_inbound.rs` parses `input-suppress <0|1>`;
`wart-arbiter-power::set_panel_blanked` fans the suppress flag to ALL hosts
alongside SetDisplayPower, riding the same blank trigger + 3 fail-safes (never
stuck). Touch-only (keys live). Verified self-driven via report-sensor sim:
cover→all hosts suppressed + injected tap dropped; uncover/call-end→resumed.

Out of scope (remaining follow-ons): underlying Android PowerManager contention;
auto-brightness via the now-reachable setDisplayBrightness; stylus/hover gating;
InputFlinger/kernel-level touch disable. Related:
[[project_arbiter_sensors]], [[feedback_no_art_layer_dependencies]],
[[reference_rsbinder_version]].
