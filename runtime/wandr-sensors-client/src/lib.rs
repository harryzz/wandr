//! wandr-sensors-client — shared `sensorservice` client (task 77).
//!
//! A CLIENT of `android.frameworks.sensorservice.ISensorManager` (the
//! frameworks-layer service present on every Android 11+ device) over rsbinder —
//! NOT a HAL client. `sensorservice` owns the single-client vendor sensors HAL
//! (`android.hardware.sensors@1.0::ISensors`) and multiplexes it, which is why
//! several wandr processes can read sensors concurrently; opening the HAL
//! directly is what caused the task-94 `DEAD_OBJECT` conflict.
//!
//! Shared by the two independent consumers: wandr-host (guest-facing `sensors`
//! WIT — per-app + ephemeral) and the arbiter's sensor-driver thread (the
//! persistent system coordinator; see task 77 on why it needs its own access).
//! Exposes **neutral** structs ([`SensorDesc`], [`SensorEvent`]) — each consumer
//! maps them to its own vocabulary (WIT `Kind`, arbiter `SensorKind`).
//!
//! Mechanism (lifted from the original `wandr-host/src/sensors_impl.rs`): a `Bn`
//! event-queue callback (`BnEventQueueCallback`) lets the service deliver `Event`
//! records into our process; the latest sample per sensor handle is accumulated
//! in a map and drained on demand. Off-android every entry point is a no-op stub
//! so callers need no `cfg` gates.

/// A sensor's static descriptor, from `getSensorList`. `aidl_type` is the raw
/// `android.hardware.sensors.SensorType` int (consumers map it; e.g. 8 =
/// proximity, 1 = accelerometer, 5 = light) so device-private sensors stay
/// visible.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SensorDesc {
    pub handle: u32,
    pub aidl_type: i32,
    /// Saturation value (proximity: the "far" distance; the threshold source).
    pub max_range: f32,
    /// Smallest reportable step.
    pub resolution: f32,
    /// Minimum sample period, microseconds (0 for on-change sensors).
    pub min_delay_us: i32,
    /// Typical power draw at full rate, mA.
    pub power_ma: f32,
}

/// One sensor reading (the payload's x/y/z scalars + the HAL timestamp). A
/// `ts_ns == 0` sentinel means "no sample" where it can occur.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SensorEvent {
    pub handle: u32,
    pub ts_ns: u64,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[cfg(target_os = "android")]
#[allow(non_snake_case, non_camel_case_types, non_upper_case_globals, dead_code, unused_imports, clippy::all)]
mod binder_aidl {
    include!(concat!(env!("OUT_DIR"), "/sensor_bindings.rs"));
}

#[cfg(target_os = "android")]
mod binder_path {
    use super::{SensorEvent, SensorDesc};
    use super::binder_aidl::android::{
        frameworks::sensorservice::{
            IEventQueue::IEventQueue,
            IEventQueueCallback::{
                BnEventQueueCallback, IEventQueueCallback, IEventQueueCallbackAsyncService,
            },
            ISensorManager::ISensorManager,
        },
        hardware::sensors::{
            Event::{Event, EventPayload},
            SensorInfo::SensorInfo as AidlSensorInfo,
        },
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};

    /// Extract x/y/z scalar values from the event's payload union. Only vec3 and
    /// scalar variants are interpreted (everything else returns zeros; the
    /// timestamp still marks the sensor alive).
    fn extract_sample(event: &Event) -> SensorEvent {
        let (x, y, z) = match &event.r#payload {
            EventPayload::EventPayload::Vec3(v) => (v.r#x, v.r#y, v.r#z),
            EventPayload::EventPayload::Vec4(v) => (v.r#x, v.r#y, v.r#z), // drop w
            EventPayload::EventPayload::Scalar(s) => (*s, 0.0, 0.0),
            EventPayload::EventPayload::Uncal(u) => (u.r#x, u.r#y, u.r#z),
            _ => (0.0, 0.0, 0.0),
        };
        SensorEvent {
            handle: event.r#sensorHandle as u32,
            ts_ns: event.r#timestamp as u64,
            x,
            y,
            z,
        }
    }

    type SampleMap = Arc<Mutex<HashMap<i32, SensorEvent>>>;

    struct EventCollector {
        latest: SampleMap,
    }
    impl rsbinder::Interface for EventCollector {}
    #[async_trait::async_trait]
    impl IEventQueueCallbackAsyncService for EventCollector {
        async fn r#onEvent(&self, event: &Event) -> rsbinder::status::Result<()> {
            let sample = extract_sample(event);
            if let Ok(mut map) = self.latest.lock() {
                map.insert(event.r#sensorHandle, sample);
            }
            Ok(())
        }
    }

    /// tokio current-thread runtime as the `BinderAsyncRuntime` (cached so only
    /// one reactor is built per process).
    struct TokioRuntime;
    impl rsbinder::BinderAsyncRuntime for TokioRuntime {
        fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
            static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
            let rt = RT.get_or_init(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio current-thread runtime")
            });
            rt.block_on(f)
        }
    }

    /// Ensure rsbinder's process-wide `ProcessState` is initialized + the binder
    /// thread pool is running before any binder call (required to talk to
    /// servicemanager AND to receive the `BnEventQueueCallback` transactions).
    /// Idempotent per process: in the arbiter (no other binder user) this does
    /// the init; in wandr-host (which inits `ProcessState` itself) the second
    /// `init_default` reports "already up" and we leave the existing pool alone.
    fn ensure_process_state() {
        static INIT: OnceLock<()> = OnceLock::new();
        INIT.get_or_init(|| {
            if !std::path::Path::new("/dev/binder").exists() {
                log::warn!("wandr-sensors-client: /dev/binder absent — sensors unavailable");
                return;
            }
            // Ok → we performed the init, so we own starting the thread pool.
            // Err → already initialized by the embedding process; its pool is up.
            if rsbinder::ProcessState::init_default().is_ok() {
                rsbinder::ProcessState::start_thread_pool();
            }
        });
    }

    /// Resolve `ISensorManager`, caching only a *successful* lookup. A `None`
    /// result is NOT cached (unlike a plain `OnceLock`), so a consumer that starts
    /// before the AIDL endpoint is registered — or survives a `sensorservice`
    /// restart — reconnects on a later call instead of staying dead forever
    /// (task 94: the arbiter's `sensor_driver` spawns at boot and must not latch a
    /// `None` if it races `wandr-sensormanager`). `Strong` is a cheap clone.
    fn service() -> Option<rsbinder::Strong<dyn ISensorManager>> {
        static SVC: OnceLock<Mutex<Option<rsbinder::Strong<dyn ISensorManager>>>> = OnceLock::new();
        let cell = SVC.get_or_init(|| Mutex::new(None));
        let mut guard = cell.lock().ok()?;
        // C3 — re-validate the cached handle: if `wandr-sensormanager` restarted, the
        // cached proxy is a dead binder. `ping_binder` it (the established pattern,
        // mirror of audio_impl.rs) and re-resolve on death so we don't latch a dead
        // handle forever. Does NOT touch the queue cache (no cross-lock) — `queue()`
        // independently pings + rebuilds its own handle.
        if let Some(svc) = guard.as_ref() {
            if svc.as_binder().ping_binder().is_ok() {
                return Some(svc.clone());
            }
            log::warn!("wandr-sensors-client: ISensorManager handle dead (wandr-sensormanager \
                        restarted?) — re-resolving");
            *guard = None;
        }
        if guard.is_none() {
            ensure_process_state();
            *guard = rsbinder::hub::get_interface::<dyn ISensorManager>(
                "android.frameworks.sensorservice.ISensorManager/default",
            )
            .ok();
        }
        guard.clone()
    }

    fn sample_map() -> SampleMap {
        static MAP: OnceLock<SampleMap> = OnceLock::new();
        MAP.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))).clone()
    }

    /// The desired-enabled sensor set (handle → rate_hz). Tracked so a queue
    /// rebuilt after a `wandr-sensormanager`/`sensorservice` restart (C3,
    /// docs/sensor-access-conflicts-no-art.md) replays the enables onto the fresh
    /// queue — otherwise sensors would stay silently dead until something
    /// re-`enable`d them.
    fn enabled_sensors() -> &'static Mutex<HashMap<u32, u32>> {
        static E: OnceLock<Mutex<HashMap<u32, u32>>> = OnceLock::new();
        E.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Sampling period (µs) for a rate; on-change sensors ignore it but still need
    /// a non-zero value. 0 Hz = HAL default (1 s).
    fn period_us(rate_hz: u32) -> i32 {
        if rate_hz == 0 { 1_000_000 } else { (1_000_000 / rate_hz).max(1) as i32 }
    }

    /// Build the event queue + register the Bn callback. Like [`service`], caches
    /// only success: if the service wasn't up yet (or `createEventQueue` failed
    /// transiently) it retries on a later call rather than latching `None`.
    fn queue() -> Option<rsbinder::Strong<dyn IEventQueue>> {
        static QUEUE: OnceLock<Mutex<Option<rsbinder::Strong<dyn IEventQueue>>>> = OnceLock::new();
        let cell = QUEUE.get_or_init(|| Mutex::new(None));
        let mut guard = cell.lock().ok()?;
        // C3 — drop a dead queue handle so it's recreated. After a wandr-sensormanager
        // restart the cached IEventQueue proxy is dead; after a sensorservice restart
        // wandr-sensormanager's internal BitTube hangs up (the event-queue stops
        // delivering and its poll thread would busy-loop). `ping_binder` catches the
        // dead-proxy case; the recreate below also re-points us at a fresh queue,
        // whose construction destroys the stale server-side EventQueue (removing the
        // hung fd from the shared looper → stops the spin).
        if let Some(q) = guard.as_ref() {
            if q.as_binder().ping_binder().is_err() {
                log::warn!("wandr-sensors-client: event queue dead — recreating + replaying enables");
                *guard = None;
            }
        }
        if guard.is_none() {
            let svc = service()?;
            let collector = EventCollector { latest: sample_map() };
            let cb: rsbinder::Strong<dyn IEventQueueCallback> =
                BnEventQueueCallback::new_async_binder(collector, TokioRuntime);
            match svc.r#createEventQueue(&cb) {
                Ok(q) => {
                    // Replay the desired-enabled set onto the fresh queue so sensors
                    // resume after a restart without anyone re-calling enable().
                    let enabled = enabled_sensors().lock().map(|m| m.clone()).unwrap_or_default();
                    for (handle, rate_hz) in &enabled {
                        if let Err(e) = q.r#enableSensor(*handle as i32, period_us(*rate_hz), 0_i64) {
                            log::warn!("wandr-sensors-client: replay enable({handle}) failed: {e:?}");
                        }
                    }
                    log::info!("wandr-sensors-client: event queue created (replayed {} enables)", enabled.len());
                    *guard = Some(q);
                }
                Err(e) => log::warn!("wandr-sensors-client: createEventQueue failed: {e:?}"),
            }
        }
        guard.clone()
    }

    pub fn enumerate() -> Vec<SensorDesc> {
        let Some(svc) = service() else { return Vec::new() };
        let Ok(list) = svc.r#getSensorList() else { return Vec::new() };
        list.into_iter()
            .map(|s: AidlSensorInfo| SensorDesc {
                handle: s.r#sensorHandle as u32,
                aidl_type: s.r#type.0,
                max_range: s.r#maxRange,
                resolution: s.r#resolution,
                min_delay_us: s.r#minDelayUs,
                power_ma: s.r#power,
            })
            .collect()
    }

    pub fn find_handle_by_type(aidl_type: i32) -> Option<u32> {
        let svc = service()?;
        let list = svc.r#getSensorList().ok()?;
        list.into_iter()
            .find(|s| s.r#type.0 == aidl_type)
            .map(|s| s.r#sensorHandle as u32)
    }

    pub fn enable(handle: u32, rate_hz: u32) -> bool {
        // Record the desired state first so a queue rebuilt later (after a restart)
        // replays it (C3). Recorded even if the call below fails transiently.
        if let Ok(mut e) = enabled_sensors().lock() {
            e.insert(handle, rate_hz);
        }
        let Some(q) = queue() else { return false };
        match q.r#enableSensor(handle as i32, period_us(rate_hz), 0_i64) {
            Ok(_) => true,
            Err(e) => {
                log::warn!("wandr-sensors-client: enableSensor({handle}, {rate_hz}Hz) err={e:?}");
                false
            }
        }
    }

    pub fn disable(handle: u32) {
        if let Ok(mut e) = enabled_sensors().lock() {
            e.remove(&handle);
        }
        let Some(q) = queue() else { return };
        let _ = q.r#disableSensor(handle as i32);
    }

    /// C3 — re-validate the binder connection, recreating the event queue (and
    /// replaying the enabled set) if `wandr-sensormanager`/`sensorservice` restarted.
    /// A no-op fast path when healthy (one `ping_binder` round-trip). The arbiter
    /// sensor-driver calls this each poll tick so sensors recover automatically.
    pub fn ensure_connected() {
        let _ = queue();
    }

    /// Remove + return the latest sample for `handle`, if a fresh one arrived.
    pub fn poll_latest(handle: u32) -> Option<SensorEvent> {
        if let Ok(mut map) = sample_map().lock() {
            return map.remove(&(handle as i32));
        }
        None
    }

    /// Remove + return every fresh sample accumulated since the last drain (used
    /// by the arbiter driver to fan all enabled sensors in one poll).
    pub fn drain_samples() -> Vec<SensorEvent> {
        if let Ok(mut map) = sample_map().lock() {
            return map.drain().map(|(_, s)| s).collect();
        }
        Vec::new()
    }
}

// ── Public API (cfg-gated; off-android = no-op stubs) ────────────────────────

/// Enumerate every sensor the HAL reports (descriptor only). Empty off-android
/// or if the service is unavailable.
pub fn enumerate() -> Vec<SensorDesc> {
    #[cfg(target_os = "android")]
    {
        binder_path::enumerate()
    }
    #[cfg(not(target_os = "android"))]
    {
        Vec::new()
    }
}

/// Find the handle of the first sensor with raw AIDL `aidl_type`, or `None`.
pub fn find_handle_by_type(aidl_type: i32) -> Option<u32> {
    #[cfg(target_os = "android")]
    {
        binder_path::find_handle_by_type(aidl_type)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = aidl_type;
        None
    }
}

/// Enable a sensor by handle at `rate_hz` (0 = HAL default / on-change). Returns
/// false off-android or on failure.
pub fn enable(handle: u32, rate_hz: u32) -> bool {
    #[cfg(target_os = "android")]
    {
        binder_path::enable(handle, rate_hz)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (handle, rate_hz);
        false
    }
}

/// Disable a sensor by handle.
pub fn disable(handle: u32) {
    #[cfg(target_os = "android")]
    {
        binder_path::disable(handle);
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = handle;
    }
}

/// Re-validate the sensor binder connection, recreating the event queue and
/// replaying the enabled sensors if `wandr-sensormanager`/`sensorservice` restarted
/// (C3, docs/sensor-access-conflicts-no-art.md). Cheap when healthy (one
/// `ping_binder`); call it periodically from a consumer that holds enabled sensors
/// but doesn't otherwise re-issue enable/disable (e.g. the arbiter sensor-driver).
/// No-op off-android.
pub fn ensure_connected() {
    #[cfg(target_os = "android")]
    {
        binder_path::ensure_connected();
    }
}

/// Poll the latest sample for `handle`, or `None` if no fresh event arrived
/// since the last poll (or off-android). Callers keep their last known value.
pub fn poll_latest(handle: u32) -> Option<SensorEvent> {
    #[cfg(target_os = "android")]
    {
        binder_path::poll_latest(handle)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = handle;
        None
    }
}

/// Drain every fresh sample across all enabled sensors since the last call.
pub fn drain_samples() -> Vec<SensorEvent> {
    #[cfg(target_os = "android")]
    {
        binder_path::drain_samples()
    }
    #[cfg(not(target_os = "android"))]
    {
        Vec::new()
    }
}
