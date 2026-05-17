use crate::bindings::my::skiko_gfx::sensors::{Host, Kind, SensorInfo, SensorSample};

// ── Binder path (Android, frameworks-layer AIDL) ─────────────────────────────
//
// Reaches `android.frameworks.sensorservice.ISensorManager/default` via
// libbinder_ndk. The frameworks-layer wrapper exists on every Android 11+
// device (it bridges to the vendor sensors HAL, which may be HIDL on older
// Pixels and stable AIDL on newer ones).
//
// First task with a real Bn-side binder server: BnEventQueueCallback wraps
// our `EventCollector` so the HAL can deliver `Event` records into our
// process. Latest sample per sensor handle is accumulated in a
// `HashMap<i32, SensorSample>`; the guest polls per frame via poll-latest().
//
// SELinux: `untrusted_app → sensorservice` is usually allowed (apps need
// sensor access), but the chain through the vendor HAL may still be denied.
// `setenforce 0` is the dev workflow on rooted devices.

#[cfg(target_os = "android")]
mod binder_path {
    use crate::binder_aidl::android::{
        frameworks::sensorservice::{
            IEventQueue::IEventQueue,
            IEventQueueCallback::{IEventQueueCallback, IEventQueueCallbackAsyncService, BnEventQueueCallback},
            ISensorManager::ISensorManager,
        },
        hardware::sensors::{
            Event::{Event, EventPayload},
            SensorInfo::SensorInfo as AidlSensorInfo,
            SensorType::SensorType as AidlSensorType,
        },
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};

    /// Map AIDL SensorType i32 → our WIT Kind. `Unknown` covers anything
    /// we don't recognize so list-sensors keeps device-private sensors
    /// visible.
    fn aidl_type_to_wit(t: AidlSensorType) -> super::Kind {
        match t.0 {
            1  => super::Kind::Accelerometer,
            2  => super::Kind::MagneticField,
            4  => super::Kind::Gyroscope,
            5  => super::Kind::Light,
            6  => super::Kind::Pressure,
            8  => super::Kind::Proximity,
            9  => super::Kind::Gravity,
            10 => super::Kind::LinearAcceleration,
            11 => super::Kind::RotationVector,
            12 => super::Kind::RelativeHumidity,
            13 => super::Kind::AmbientTemperature,
            15 => super::Kind::GameRotationVector,
            _  => super::Kind::Unknown,
        }
    }

    /// Extract x/y/z scalar values from the event's payload union. Only
    /// vec3 and scalar variants are interpreted; everything else returns
    /// zeros (the timestamp is still useful — guest sees a non-sentinel
    /// reading and can know the sensor is alive).
    fn extract_sample(event: &Event) -> super::SensorSample {
        let (x, y, z) = match &event.r#payload {
            EventPayload::EventPayload::Vec3(v)    => (v.r#x, v.r#y, v.r#z),
            EventPayload::EventPayload::Vec4(v)    => (v.r#x, v.r#y, v.r#z),  // drop w
            EventPayload::EventPayload::Scalar(s)  => (*s, 0.0, 0.0),
            EventPayload::EventPayload::Uncal(u)   => (u.r#x, u.r#y, u.r#z),
            _ => (0.0, 0.0, 0.0),
        };
        super::SensorSample {
            timestamp_ns: event.r#timestamp as u64,
            x, y, z,
        }
    }

    type SampleMap = Arc<Mutex<HashMap<i32, super::SensorSample>>>;

    struct EventCollector { latest: SampleMap }
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

    /// tokio current-thread runtime as the `BinderAsyncRuntime`. Needed by
    /// `BnEventQueueCallback::new_async_binder`. The runtime is cached so
    /// only one tokio reactor is built per process.
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

    /// Lazy global state — initialized on the first WIT call. Splitting
    /// out the queue/callback init from list-sensors lets list-sensors
    /// stay cheap when the guest doesn't actually enable anything.
    fn service() -> Option<&'static rsbinder::Strong<dyn ISensorManager>> {
        static SVC: OnceLock<Option<rsbinder::Strong<dyn ISensorManager>>> = OnceLock::new();
        SVC.get_or_init(|| {
            rsbinder::hub::get_interface::<dyn ISensorManager>(
                "android.frameworks.sensorservice.ISensorManager/default"
            ).ok()
        }).as_ref()
    }

    fn sample_map() -> SampleMap {
        static MAP: OnceLock<SampleMap> = OnceLock::new();
        MAP.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))).clone()
    }

    /// Build the event queue + register the Bn callback exactly once.
    /// Returns None if the service is unavailable or callback registration
    /// failed.
    fn queue() -> Option<&'static rsbinder::Strong<dyn IEventQueue>> {
        static QUEUE: OnceLock<Option<rsbinder::Strong<dyn IEventQueue>>> = OnceLock::new();
        QUEUE.get_or_init(|| {
            let svc = service()?;
            let collector = EventCollector { latest: sample_map() };
            let cb: rsbinder::Strong<dyn IEventQueueCallback> =
                BnEventQueueCallback::new_async_binder(collector, TokioRuntime);
            match svc.r#createEventQueue(&cb) {
                Ok(q) => {
                    log::info!("sensors: event queue created");
                    Some(q)
                }
                Err(e) => {
                    log::warn!("sensors: createEventQueue failed: {e:?}");
                    None
                }
            }
        }).as_ref()
    }

    pub fn list_sensors() -> Vec<super::SensorInfo> {
        let Some(svc) = service() else { return Vec::new() };
        let Ok(list) = svc.r#getSensorList() else { return Vec::new() };
        list.into_iter()
            .map(|s: AidlSensorInfo| super::SensorInfo {
                handle:       s.r#sensorHandle as u32,
                kind:         aidl_type_to_wit(s.r#type),
                max_range:    s.r#maxRange,
                resolution:   s.r#resolution,
                // SensorInfo.minDelayUs is microseconds; convert to ms,
                // clamp to >= 1 so a guest divider never sees 0. On-change
                // sensors return 0 (no period); we expose as 1 ms.
                min_delay_ms: ((s.r#minDelayUs / 1000).max(1)) as u32,
                power_ma:     s.r#power,
            })
            .collect()
    }

    pub fn enable(handle: u32, rate_hz: u32) -> bool {
        let Some(q) = queue() else { return false };
        let period_us = if rate_hz == 0 { 1_000_000 } else { (1_000_000 / rate_hz).max(1) };
        match q.r#enableSensor(handle as i32, period_us as i32, 0_i64) {
            Ok(_)  => true,
            Err(e) => { log::warn!("sensors: enableSensor({handle}, {rate_hz}Hz) err={e:?}"); false }
        }
    }

    pub fn disable(handle: u32) {
        let Some(q) = queue() else { return };
        let _ = q.r#disableSensor(handle as i32);
    }

    pub fn poll_latest(handle: u32) -> super::SensorSample {
        if let Ok(mut map) = sample_map().lock() {
            if let Some(s) = map.remove(&(handle as i32)) {
                return s;
            }
        }
        // Sentinel
        super::SensorSample { timestamp_ns: 0, x: 0.0, y: 0.0, z: 0.0 }
    }
}

impl Host for crate::HostState {
    fn list_sensors(&mut self) -> Vec<SensorInfo> {
        #[cfg(target_os = "android")]
        { return binder_path::list_sensors(); }
        #[cfg(not(target_os = "android"))]
        { Vec::new() }
    }

    fn enable(&mut self, handle: u32, rate_hz: u32) -> bool {
        #[cfg(target_os = "android")]
        { return binder_path::enable(handle, rate_hz); }
        #[cfg(not(target_os = "android"))]
        { let _ = (handle, rate_hz); false }
    }

    fn disable(&mut self, handle: u32) {
        #[cfg(target_os = "android")]
        binder_path::disable(handle);
        #[cfg(not(target_os = "android"))]
        { let _ = handle; }
    }

    fn poll_latest(&mut self, handle: u32) -> SensorSample {
        #[cfg(target_os = "android")]
        { return binder_path::poll_latest(handle); }
        #[cfg(not(target_os = "android"))]
        {
            let _ = handle;
            SensorSample { timestamp_ns: 0, x: 0.0, y: 0.0, z: 0.0 }
        }
    }
}

// Silence unused-import warning on desktop where the binder path is gone.
#[cfg(not(target_os = "android"))]
#[allow(dead_code)]
fn _unused(k: Kind) { let _ = k; }
