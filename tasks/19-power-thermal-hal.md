# Task 19 — IPower + IThermal HALs

> **Status: ✅ device-verified on Pixel 2 XL 2026-05-17.** Both HALs reachable via the rsbinder pipeline. Smoke output: `hint(INTERACTION) supported=true, boost sent, overallThrottle=NONE, sensors=11 first=CPU/39.0°C`. Required (a) bumping the AOSP HAL AIDL submodule from `android-11.0.0_r48` to `android-15.0.0_r36` (stable AIDL thermal didn't exist before Android 13), (b) adding `common/aidl` + `common/fmq/aidl` to sparse-checkout for transitive imports, (c) stubbing `android.os.PersistableBundle` in `vendor/aidl-stubs/` because the newer vibrator AIDL references it, (d) splitting WIT `list-temperatures(filter: option<kind>)` into two methods to avoid hand-rolling the option<enum> arg ABI, and (e) byte-reading the u8-backed enum fields with the correct 12-byte stride for the record layout (canonical ABI keeps WIT field order, doesn't reorder like Rust does — Rust's `size_of::<Temperature>() = 8` is a misleading host-side number).

## Goal

Two small additions to the host's HAL surface, both useful immediately:

1. **IPower performance hints** — Compose animations + scroll can ask the HAL "user is interacting" / "frame about to render" so the CPU governor doesn't down-clock mid-frame. Direct measurable smoothness improvement on heavy widgets (LazyColumn, ProgressIndicator).

2. **IThermal read-only state** — apps can detect "device is hot" and degrade gracefully (drop animations, lower frame rate, simpler UI). Useful for sustained workloads (video playback, gaming-style UIs).

Both bound at WIT layer in domain terms — the guest never sees AIDL `PowerHint` enums or `TemperatureType` ints.

Reference: `~/wandr/post-art-roadmap.md` §3, follows tasks 15 (pipeline) + 16 (vibrator pattern).

---

## Architecture

Same shape as task 16 (vibrator):

```
WIT power.hint(kind, duration_ms)         WIT thermal.list-temperatures(filter)
  └─ Host::hint()                            └─ Host::list_temperatures()
       └─ binder_path::hint()                     └─ binder_path::list_temperatures()
            └─ svc.r#sendHint(...)                     └─ svc.r#getCurrentTemperatures(...)
                 (IPower binder)                            (IThermal binder)
```

Neither IPower nor IThermal's read paths take `@nullable` parameters → the **generated rsbinder-aidl proxies work directly** (no manual-parcel workaround like task 16 needed). IThermal's `registerThermalChangedCallback` does take a callback but we don't need it for the read-only WIT — defer until we expose a thermal-listener WIT.

## WIT design (proposed — refine when writing host impl)

```wit
/// Performance hints to the kernel governor. The HAL may ignore unknown
/// hints or coalesce repeated ones — calling `hint(interaction, 100)`
/// every frame during a scroll is the expected pattern.
interface power {
    enum hint {
        interaction,             // user is actively touching/scrolling
        display-update-imminent, // frame about to swap
        sustained-performance,   // long-running compute (rare in Compose UIs)
        expensive-rendering,     // heavy frame (gradients, blur)
        launch,                  // app cold-start
    }
    enum mode {
        low-power,               // battery saver
        sustained-performance,   // anti-throttle for steady workloads
        fixed-performance,       // pin frequency (benchmark/test only)
        expensive-rendering,     // sustained heavy GPU
    }
    /// Send a one-shot hint. `duration-ms` is advisory (0 = HAL default).
    hint:     func(kind: hint, duration-ms: u32);
    /// Toggle a sustained mode on/off. Pairs of enable/disable expected.
    set-mode: func(kind: mode, enabled: bool);
}

/// Read-only thermal state. Listener-style WIT deferred to a follow-up
/// (would require Bn-side callback infrastructure similar to task 16).
interface thermal {
    enum kind { cpu, gpu, battery, skin, modem, npu, display }
    enum throttle {
        %none,      // 0 = normal
        light,      // 1
        moderate,   // 2
        severe,     // 3
        critical,   // 4
        emergency,  // 5
        shutdown,   // 6
    }
    record temperature {
        kind:     kind,
        celsius:  f32,
        throttle: throttle,
    }
    /// Return all temperature sensors of the given kind (or every sensor
    /// if filter is none). Empty list on devices without that sensor.
    list-temperatures: func(filter: option<kind>) -> list<temperature>;
    /// Most severe throttle status across all sensors — what apps care
    /// about for "should I degrade my UI?" decisions.
    overall-throttle: func() -> throttle;
}
```

Both enums (`hint`/`mode`/`kind`/`throttle`) map 1:1 to AIDL ordinal positions in the r48 enums, so the WIT→AIDL translation in the Rust host is a straight `match`.

---

## Steps

### 1. Expand sparse-checkout for power + thermal AIDL

```bash
cd ~/wandr/wandr-host/vendor/aosp-hardware-interfaces
git sparse-checkout set vibrator/aidl light/aidl power/aidl thermal/aidl
ls power/aidl/android/hardware/power/IPower.aidl
ls thermal/aidl/android/hardware/thermal/IThermal.aidl
```

No submodule re-add needed — same `aosp-hardware-interfaces` at pinned `android-11.0.0_r48`, just more files visible.

### 2. Add to `build.rs` rsbinder-aidl Builder

In the existing `if target_os == "android"` block:
- Add `.source(...IPower.aidl)` and `.source(...IThermal.aidl)`
- IPower has parcelables `IPowerCallback` (unused for read-only) and supporting types — `include_dir` already covers them
- IThermal has parcelables `Temperature`, `TemperatureType`, `ThrottlingSeverity`, `CoolingDevice`, `CoolingType` + callback `IThermalChangedCallback` (unused for read-only)

### 3. New file `wandr-host/src/power_impl.rs`

- `binder_path::hint(kind, duration_ms)` → `svc.r#sendHint(aidl_hint, duration_ms as i32)`
- `binder_path::set_mode(kind, enabled)` → `svc.r#setMode(aidl_mode, enabled)`
- `OnceLock<Option<Strong<dyn IPower>>>` for service handle (one binder lookup per process)
- WIT→AIDL enum mapping in explicit `match` (same pattern as task 17 lights)

`wandr-host/src/lib.rs` — add `mod power_impl;`

### 4. New file `wandr-host/src/thermal_impl.rs`

- `binder_path::list_temperatures(filter)` → `svc.r#getCurrentTemperatures(matches_filter_flag, kind_or_0)` returns `Vec<Temperature>` → map to WIT records
- `binder_path::overall_throttle()` → iterate `getCurrentTemperatures` filter=all, return max `throttlingStatus`
- Same `OnceLock` cache pattern

### 5. New WIT entries

- Append `interface power { ... }` and `interface thermal { ... }` to `wit/skiko-gfx.wit` after the existing `lights` block
- Add `import power; import thermal;` to `world skiko-ui`
- `cp` to skiko mirror

### 6. Hand-edit Kotlin bindings

Per task 17's documented pattern, mirror the `Lights` block for both `Power` and `Thermal` in `SkikoUi.kt` + add `@WasmImport` extern declarations in `InternalSkikoUi.kt`.

### 7. Verification

- `cargo build --target aarch64-linux-android --release` succeeds
- `gradle compileProductionExecutableKotlinWasmWasi` succeeds
- Build + deploy + restart per CLAUDE.md pipeline
- Device verify (Pixel 2 XL, `setenforce 0`):
  - Add temp Main.kt smoke: `Power.Import.hint(Hint.INTERACTION, 100u)` + `Thermal.Import.list_temperatures(null) → expect 1+ entries with sensible celsius values for at least CPU and battery sensors`
  - Logcat should show the binder transactions succeed
  - Remove temp test before commit

---

## Known issues / risks

1. **SELinux scope.** `untrusted_app → hal_power_default` and `→ hal_thermal_default` denials still apply. Same `setenforce 0` workflow as task 16. Production fix waits for roadmap §6.1 boot-model work.

2. **AIDL version drift.** IPower added new hint codes (`CPU_LOAD_UP`, `CPU_LOAD_RESET` etc) in later Android releases; our r48 vendored AIDL is the older surface. If a Pixel 6 Pro HAL sends an unknown enum value our deserializer should error gracefully rather than panic — test with `getCurrentTemperatures` response decoding on a newer device when available.

3. **No callback paths exposed.** `IThermalChangedCallback` and `IPowerCallback` are not wrapped. Adding listener-WIT later would re-encounter the `@nullable` workaround pattern from task 16 (manual parcel + Option::<Strong>::None) — except thermal/power callbacks aren't nullable, so a real Bn-side callback IS needed (would need to revisit the NopCallback/BinderAsyncRuntime path that didn't ship for vibrator).

4. **Hint coalescing semantics differ by vendor.** Some HALs treat repeated `sendHint(INTERACTION)` as a stream; others throttle to ~10 Hz; some require pairs of `enable_hint` / `disable_hint`. Document observed behavior on the test device.

---

## Out of scope

- Listener-style WIT for thermal changes (separate follow-up; needs Bn-callback infra).
- `IThermal.getCurrentCoolingDevices` / `getThermalHeadroom` — niche, defer.
- `IPower.getHintSessionPreferredRate` / `createHintSession` (per-thread CPU governor sessions, Android 13+) — newer API, our r48 AIDL doesn't have it.
- Compose integration — Compose has no built-in `power.hint()` primitive. Exposing to guest apps is a separate guest-API design decision.
