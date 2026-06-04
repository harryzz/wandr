//! The arbiter's sensor-driver thread (task 77) — the ONLY place that touches
//! the sensor HAL. Mirrors `spawn_alarm_timer` / `spawn_screen_poller`: a
//! background thread that drains samples off the HAL event queue and
//! `bus_emit`s `Event::SensorReading`, plus a `set_sensor` entry the effect
//! executor calls to apply the pure module's `Effect::SetSensor` (enable /
//! disable on demand — the battery contract).
//!
//! The binder mechanism lives in the shared `wart-hal-sensors` crate (also used
//! by wart-host); this file owns only the arbiter-side wiring: the kind↔handle
//! maps (built once from `enumerate`), the descriptor seed into the core Store
//! (so the sensors module derives the proximity threshold from real hardware),
//! and the poll→`bus_emit` loop.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use wart_arbiter_core::SensorKind;
use wart_hal_sensors::HalSensor;

/// How often the driver drains the HAL event queue while ≥1 sensor is enabled.
/// On-change sensors (proximity) push events to our callback asynchronously; this
/// only bounds the latency from a HAL crossing to the bus. 200 ms is well under
/// human reaction time for screen-off-on-ear while staying cheap.
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// Idle tick when nothing is enabled (no draining — just re-checks the count).
const IDLE_INTERVAL: Duration = Duration::from_millis(1000);

/// Recognized AIDL `SensorType` → arbiter [`SensorKind`]. Mirrors the host's
/// mapping; unrecognized sensors are ignored (the arbiter only arbitrates the
/// kinds a consumer asks for).
fn aidl_type_to_kind(aidl_type: i32) -> Option<SensorKind> {
    match aidl_type {
        1 => Some(SensorKind::Accelerometer),
        5 => Some(SensorKind::Light),
        8 => Some(SensorKind::Proximity),
        _ => None,
    }
}

/// `kind → HalSensor` for every recognized sensor, built once at `spawn`.
fn sensors() -> &'static HashMap<SensorKind, HalSensor> {
    static MAP: OnceLock<HashMap<SensorKind, HalSensor>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        for s in wart_hal_sensors::enumerate() {
            if let Some(kind) = aidl_type_to_kind(s.aidl_type) {
                // First sensor of each kind wins (devices rarely report duplicates).
                m.entry(kind).or_insert(s);
            }
        }
        m
    })
}

/// Reverse `handle → kind` for the poll loop, derived from [`sensors`].
fn handle_to_kind() -> &'static HashMap<u32, SensorKind> {
    static MAP: OnceLock<HashMap<u32, SensorKind>> = OnceLock::new();
    MAP.get_or_init(|| sensors().iter().map(|(k, s)| (s.handle, *k)).collect())
}

/// Number of currently-enabled sensors — gates the poll loop so it does no work
/// (and takes no locks) while nothing is enabled.
static ENABLED: AtomicUsize = AtomicUsize::new(0);

/// Apply the sensors module's enable/disable decision to the HAL (called from
/// `execute_effects`, which holds `arbiter_lock`). Looks up the handle for
/// `kind` and toggles it; updates the enabled count that gates the poll loop.
pub fn set_sensor(kind: SensorKind, on: bool, rate_hz: u32) {
    let Some(sensor) = sensors().get(&kind) else {
        log::warn!("sensor_driver: SetSensor {} but no such sensor on this device", kind.as_wire());
        return;
    };
    if on {
        if wart_hal_sensors::enable(sensor.handle, rate_hz) {
            ENABLED.fetch_add(1, Ordering::Relaxed);
            log::info!("sensor_driver: enabled {} (handle {})", kind.as_wire(), sensor.handle);
        }
    } else {
        wart_hal_sensors::disable(sensor.handle);
        // Saturating decrement (never below 0 if a disable arrives unmatched).
        let _ = ENABLED.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| Some(n.saturating_sub(1)));
        log::info!("sensor_driver: disabled {} (handle {})", kind.as_wire(), sensor.handle);
    }
}

/// Spawn the HAL poll thread. Enumerates sensors, seeds their descriptors into
/// the core Store (so the sensors module can derive the proximity threshold from
/// real `max_range`/`resolution`), then loops draining samples → `bus_emit`.
pub fn spawn() {
    // Build the maps + seed descriptors up front (so a `report-sensor` sim verb
    // or the first real consumer has thresholds before any sample arrives).
    let known = sensors();
    if known.is_empty() {
        log::warn!("sensor_driver: no recognized sensors enumerated (HAL unavailable?) — driver idle");
    } else {
        let _guard = crate::arbiter_lock().lock().unwrap_or_else(|e| e.into_inner());
        let mut store = crate::core_store().lock().unwrap_or_else(|e| e.into_inner());
        for (kind, s) in known {
            store.set_sensor_descriptor(*kind, s.max_range, s.resolution);
            log::info!(
                "sensor_driver: {} handle={} max_range={} resolution={}",
                kind.as_wire(), s.handle, s.max_range, s.resolution
            );
        }
    }

    std::thread::Builder::new()
        .name("arbiter-sensor-driver".into())
        .spawn(|| loop {
            if ENABLED.load(Ordering::Relaxed) == 0 {
                std::thread::sleep(IDLE_INTERVAL);
                continue;
            }
            std::thread::sleep(POLL_INTERVAL);
            let samples = wart_hal_sensors::drain_samples();
            if samples.is_empty() {
                continue;
            }
            let map = handle_to_kind();
            let _guard = crate::arbiter_lock().lock().unwrap_or_else(|e| e.into_inner());
            for s in samples {
                let Some(kind) = map.get(&s.handle).copied() else { continue };
                crate::bus_emit(wart_arbiter_core::Event::SensorReading {
                    kind,
                    x: s.x,
                    y: s.y,
                    z: s.z,
                    ts_ns: s.ts_ns,
                });
            }
        })
        .expect("spawn sensor driver thread");
}
