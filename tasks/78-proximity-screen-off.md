# Task 78 — proximity screen-off during calls (task-77 follow-on)

> Status: ✅ DONE — device-verified on Pixel 2 XL (2026-06-04). Cover the
> proximity sensor during a call → panel **off** (SurfaceFlinger `setPowerMode`);
> uncover or hang up → panel **on**. Fail-safe verified: ending the call while
> still covered restores the panel (never stuck dark).

## What this delivers

The applier task 77 deferred: `Event::ProximityChanged` now drives a real
panel-off (the HWC powers the panel down — deeper than a backlight write) while a
call is active, instead of just logging "would blank now".

## Mechanism (de-risked on-device before building)

SurfaceFlinger `android.gui.ISurfaceComposer.setPowerMode(token, mode)`, reached
over rsbinder. Confirmed by probing the device:
- `SurfaceFlingerAIDL` reachable as root; transaction order matches the full
  upstream interface — `getPhysicalDisplayIds`(6), `getPhysicalDisplayToken`(7),
  **`setPowerMode`(8)**. (The old 4-method stub was mis-tuned, calling
  `createVirtualDisplay` at code 4 — its NPE was misread as "round-trip OK".)
- Display token (id 0) obtainable as root; `setPowerMode` shares
  `getPhysicalDisplayToken`'s exact permission gate (`checkAccessPermission` →
  `callingThreadHasUnscopedSurfaceFlingerAccess`), which **succeeds as root**.
- `hal::PowerMode`: OFF=0, ON=2.

## Implementation

- **`runtime/gui-aidl-types-rs/`** — NEW shim crate (`crate_name gui_aidl_types_rs`):
  9 zero-sized stub parcelables (rsbinder `Parcelable` + `impl_serialize/deserialize_for_parcelable!`,
  `unimplemented!()` bodies). The full `ISurfaceComposer` declares these as
  `rust_type "gui_aidl_types_rs::X"` native-backed types; AOSP's own crate is
  likewise stubs. None are on the `setPowerMode` path.
- **`runtime/wart-hal-display/`** — NEW shared crate (mirrors `wart-hal-sensors`):
  `build.rs` codegens the **full, un-trimmed** `ISurfaceComposer` from the vendored
  extract AIDL — copies the closure into `OUT_DIR`, applies a `0f`→`0` float-literal
  fix there (never mutating vendored files), resolves the native types to the shim.
  API `set_display_power(on)`: derives the primary display id from
  `getPhysicalDisplayIds` (no hardcoded 0) → `getPhysicalDisplayToken` →
  `setPowerMode(token, ON/OFF)`. `ensure_process_state()` like wart-hal-sensors.
- **core** (`wart-arbiter-core`): `Effect::SetDisplayPower { on }`.
- **`wart-arbiter-power`**: `blanked` flag; on `ProximityChanged` blank only while
  a call is active (toggle on transition); **3 fail-safes** force the panel back on
  (`!near`, `CommsActive{false}` with no calls left, `SurfaceRemoved` of the last
  call). 3 new unit tests.
- **`wart-arbiter-bin`**: `Effect::SetDisplayPower` arm in `execute_effects` →
  `wart_hal_display::set_display_power`.

## Verification

- Unit (9 power + 13 core, host): blank only during a call; transition-only; all
  three fail-safes restore the panel.
- Device: `audio-call-start war.signal` → proximity auto-enabled (task-77 wiring);
  cover → `panel OFF` + `SetDisplayPower on=false applied=true` (screen visually
  dark, user-confirmed); uncover → `panel ON`; `audio-call-end` while covered →
  `proximity blank cleared → panel ON` + sensor disabled (battery). `setPowerMode`
  `applied=true` throughout (root permission as predicted).

## Out of scope (follow-ons)

- **Touch suppression during blank** — `setPowerMode(OFF)` powers the panel down
  but the touch controller (InputFlinger) is separate; a cheek/ear touch can still
  register. Follow-up if it bites.
- **Underlying Android PowerManager contention** — OS PM still governs idle display
  power; our forced toggle is brief (call duration). Revisit if PM fights it.
- Adopting `wart-hal-display` in the host's `display_impl.rs`; auto-brightness via
  the now-reachable `setDisplayBrightness`.
