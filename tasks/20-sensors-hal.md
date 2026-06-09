# Task 20 — Sensors (via frameworks-layer ISensorManager)

> **Status: ✅ device-verified on Pixel 2 XL 2026-05-17.** Smoke output: `29 sensors enumerated; accel handle=1; pollLatest x=-1.21 y=0.003 z=9.61 (m/s²)` — z≈9.81 confirms gravity dominating, real readings flowing from sensorservice → Bn callback → our HashMap → guest poll. **First task with a working Bn-side binder server** — `BnEventQueueCallback::new_async_binder(EventCollector, TokioRuntime)` registered cleanly (no `unknown native id` errors that bit us in task 16's earlier callback attempts). Targets the frameworks-layer stable AIDL wrapper rather than the vendor sensors HAL — this is intentional and proven: the wrapper exists on every Android 11+ device while the vendor HAL is HIDL on Pixel 2 XL.

## Goal

Expose Android sensors to WASM Compose apps. Most-wanted use cases: orientation-aware UIs (compass rose, AR overlays), motion-triggered actions (shake-to-undo), ambient-light-driven dark mode, proximity-based screen sleep.

Reference: `~/wandr/post-art-roadmap.md` §3. Builds on the rsbinder pipeline (task 15) and the lessons from task 16 (callback-server-side infrastructure).

---

## Architecture

`android.frameworks.sensorservice.ISensorManager` is JetBrains' frameworks-layer wrapper around the vendor HIDL `android.hardware.sensors@1.0::ISensors` — runs in `sensorservice` daemon. Stable AIDL exposed everywhere; we don't have to think about HIDL.

```
WIT sensors.list-sensors() → Vec<SensorInfo>
WIT sensors.enable(handle, rate_hz) → bool
WIT sensors.poll-latest(handle) → Option<SensorSample>
WIT sensors.disable(handle)
  └─ Host::*
       └─ binder_path::*
            ├─ svc.r#getSensorList()
            ├─ svc.r#createEventQueue(BnEventQueueCallback)  // ← callback-heavy
            ├─ event_queue.r#enableSensor(handle, period_us, batch_us)
            └─ event_queue.r#disableSensor(handle)

  Host runs a Bn-side IEventQueueCallback that accumulates the
  latest sample per sensor handle into a HashMap<u32, SensorSample>.
  Guest polls via poll-latest() — no callback crosses the WIT boundary.
```

**Pull model vs push model.** The natural sensor API in Android is push (event queue calls back). Exposing push at the WIT layer would require a guest export `on-sensor-event` similar to existing input/pointer exports. Pull is simpler:

- Host owns the IEventQueueCallback Bn binder + a `HashMap<u32, SensorSample>`
- Each `onEvent(events)` callback overwrites the latest sample per sensor handle (drops older samples)
- Guest polls per-frame: `sensors.poll-latest(accel_handle)` returns the most recent reading or None if nothing arrived since the last poll

The push model fits Compose's per-frame redraw rhythm: guest is going to redraw the frame anyway, polling for the latest sample is the right cadence. Push-via-WIT-export would force the runtime to bridge sensor-thread → wasm-main-thread synchronization for no UX benefit (Compose can't render mid-frame anyway).

## WIT design (proposed — refine when writing host impl)

```wit
interface sensors {
    enum kind {
        accelerometer, gyroscope, magnetic-field,
        proximity, light, pressure,
        gravity, linear-acceleration, rotation-vector,
        ambient-temperature, relative-humidity,
        step-counter, step-detector,
        game-rotation-vector, heart-rate,
    }
    record sensor-info {
        handle:        u32,    // opaque ID for enable/disable/poll
        kind:          kind,
        name:          string, // vendor-given (e.g. "BMI160 Accelerometer")
        vendor:        string,
        max-range:     f32,    // sensor-specific units
        resolution:    f32,
        min-delay-ms:  u32,    // shortest supported period
        power-ma:      f32,    // estimated current draw when active
    }
    record sensor-sample {
        timestamp-ns: u64,
        /// Length depends on `kind`:
        ///   - 1: light, proximity, pressure, ambient-temperature, relative-humidity,
        ///        step-counter, step-detector, heart-rate
        ///   - 3: accelerometer, gyroscope, magnetic-field, gravity,
        ///        linear-acceleration
        ///   - 4: rotation-vector, game-rotation-vector (quaternion w/x/y/z)
        values:       list<f32>,
    }
    /// Enumerate every sensor the device exposes. Cheap, cache once on app start.
    list-sensors: func() -> list<sensor-info>;
    /// Start streaming samples at the requested rate in Hz. Returns false if
    /// the sensor handle is unknown, rate is unsupported, or the HAL refused.
    /// Calling enable() a second time on the same handle changes the rate.
    enable:       func(handle: u32, rate-hz: u32) -> bool;
    /// Stop streaming. Safe to call on a not-enabled handle (no-op).
    disable:      func(handle: u32);
    /// Latest sample for `handle`, or none if no sample has arrived since the
    /// last poll. Calling poll-latest() consumes the sample (subsequent calls
    /// return none until the next event).
    poll-latest:  func(handle: u32) -> option<sensor-sample>;
}
```

`kind` enum covers the ~15 most-used Android sensor types. Less common ones (`step_counter`, `geomagnetic_rotation_vector`, etc.) deliberately omitted — add only when a real use case appears. Unknown vendor sensor types map to none (filtered from `list-sensors`).

---

## Steps

### 1. Vendor a new submodule for frameworks-layer AIDLs

`android.frameworks.sensorservice` lives in `platform/frameworks/hardware/interfaces`, **NOT** the existing `platform/hardware/interfaces`. Add a sibling submodule:

```bash
cd ~/wandr/wandr-host
git submodule add --depth 1 \
    https://android.googlesource.com/platform/frameworks/hardware/interfaces \
    vendor/aosp-frameworks-hardware-interfaces
cd vendor/aosp-frameworks-hardware-interfaces
git fetch --depth 1 origin refs/tags/android-11.0.0_r48:refs/tags/android-11.0.0_r48
git checkout -f android-11.0.0_r48
git sparse-checkout init --cone
git sparse-checkout set sensorservice/aidl
```

Set `shallow = true` in `.gitmodules` alongside the existing entries.

### 2. Add to `build.rs` rsbinder-aidl Builder

```rust
let frameworks_vendor = PathBuf::from("vendor/aosp-frameworks-hardware-interfaces");
let sensorservice_aidl = frameworks_vendor.join("sensorservice/aidl");
rsbinder_aidl::Builder::new()
    .source(sensorservice_aidl.join("android/frameworks/sensorservice/ISensorManager.aidl"))
    .source(sensorservice_aidl.join("android/frameworks/sensorservice/IEventQueue.aidl"))
    .include_dir(sensorservice_aidl)
    .set_async_support(true)
    .output(...)  // separate output file or append to aosp_hal_bindings.rs
    .generate()
    .expect(...);
```

`IEventQueueCallback.aidl` is needed for the callback path — it'll be pulled in via include_dir.

### 3. Bn-side IEventQueueCallback (callback server)

The hard part of this task. ISensorManager.createEventQueue **requires** a non-null callback (no `@nullable` like vibrator). We need a real binder server on our side.

Two implementation paths:

- **(A) NopCallback + BinderAsyncRuntime path** (the approach that didn't ship in task 16 because vibrator's HAL refused callbacks). Sensors *will* fire callbacks — that's the whole point — so we need a working server. Use tokio current-thread runtime (already pulled in transitively by rsbinder's `tokio` default feature). Wrap NopCallback in `BnEventQueueCallback::new_async_binder(EventCollector, runtime)` where `EventCollector` is our actual sensor-sample-storing impl.
- **(B) Lower-level binder primitives** — register a custom binder with manual `transact` dispatching. More code, more debugging. Defer unless (A) hits trouble.

Recommend (A). The `EventCollector` impl:

```rust
struct EventCollector {
    latest: Arc<Mutex<HashMap<u32, SensorSample>>>,
}
#[async_trait::async_trait]
impl IEventQueueCallbackAsyncService for EventCollector {
    async fn r#onEvent(&self, events: Vec<Event>) -> rsbinder::status::Result<()> {
        let mut map = self.latest.lock().unwrap();
        for e in events {
            map.insert(e.sensorHandle as u32, to_wit_sample(e));
        }
        Ok(())
    }
}
```

### 4. New file `wandr-host/src/sensors_impl.rs`

- `OnceLock<Strong<dyn ISensorManager>>` for service
- One `Strong<dyn IEventQueue>` per process (or per enabled sensor — TBD; ISensorManager.createEventQueue() returns one queue we can multiplex sensors onto)
- `latest: Arc<Mutex<HashMap<u32, SensorSample>>>` for sample storage
- `list_sensors()` calls `svc.r#getSensorList()` once, caches result
- `enable(handle, rate_hz)` calls `event_queue.r#enableSensor(handle, period_us, batch_us)` where `period_us = 1_000_000 / rate_hz`
- `disable(handle)` calls `event_queue.r#disableSensor(handle)`
- `poll_latest(handle)` does `map.lock().unwrap().remove(&handle)` (atomic take, returns option)

Map AIDL `SensorInfo.type` int values (from `sensors-base.h`) to our `kind` enum; filter out unknown.

### 5. WIT + Kotlin bindings + lib.rs wiring

- Append `interface sensors { ... }` to `wit/skiko-gfx.wit`, add to world
- Sync mirror
- Hand-edit Kotlin bindings (mirror the `Lights` pattern; `sensor-info` and `sensor-sample` records have variable-length string + list fields — non-trivial pointer-based serialization, harder than the flat enums in task 17)
- `mod sensors_impl;` in lib.rs

### 6. Verification

- Build chain succeeds (Rust + gradle)
- Deploy + restart
- Smoke test in Main.kt:
  ```kotlin
  val list = Sensors.Import.listSensors()
  WitCanvas.Import.logMessage("sensors: ${list.size} found")
  val accel = list.find { it.kind == Sensors.Kind.ACCELEROMETER } ?: return
  Sensors.Import.enable(accel.handle, 50u)
  // ... wait some frames ...
  val sample = Sensors.Import.pollLatest(accel.handle)
  WitCanvas.Import.logMessage("sensors: accel=${sample?.values}")
  ```
- Expect on Pixel 2 XL: many sensors listed (Pixel 2 has accel, gyro, mag, proximity, light, pressure, fingerprint hardware sensors, etc.). Accel sample like `[0.0, 9.8, 0.0]` when phone is flat on table.

---

## Known issues / risks

1. **SELinux.** `untrusted_app → sensorservice` is OFTEN allowed in stock policy (apps need sensor access) but our binder path goes through the *vendor* binder driver — verify denials in dmesg on first test.

2. **Callback server is the bulk of the work.** This is the first time we're shipping a real Bn-side binder in WAR (task 16's NopCallback was never used). Pattern needs to be solid — future tasks will reuse it for any HAL that needs callbacks (thermal listener, location updates, biometrics, etc.).

3. **Variable-length record marshalling.** `sensor-info` has 2 strings + 5 numerics; `sensor-sample` has a `list<f32>` (1, 3, or 4 elements). Component-model canonical ABI passes these via memory pointer, not flat args. Kotlin marshalling for records this complex hasn't been hand-written in our generated bindings yet — copy patterns from the existing `PaintAttrs` codegen.

4. **Sample-rate vs power.** Many sensors (accel @ 100 Hz) draw measurable battery. Add safety: cap `rate_hz` at the sensor's reported `min-delay-ms` reciprocal; warn in logcat if guest asks for higher than supported.

5. **Sensor handle stability.** Handles returned by `getSensorList()` are stable within a sensorservice lifetime but reset across reboots. Don't persist handles across app restarts in WIT contract — `list_sensors()` is the source of truth on each launch.

---

## Out of scope

- Direct-channel sensors (`createAshmemDirectChannel`) for high-rate streams (gyro at 1 kHz) — niche, defer.
- Trigger sensors (`SignificantMotion`, one-shot events) — rare use case, separate WIT method later.
- Sensor calibration / bias removal — apps that need it can do it themselves on the raw samples.
- Push-model `on-sensor-event` WIT export — pull model fits Compose's per-frame redraw better. Reconsider only if a real use case demands sub-frame latency.
