//! The arbiter's sensor-driver thread (task 77) — the ONLY place that touches
//! the sensor service. Mirrors `spawn_alarm_timer` / `spawn_screen_poller`: a
//! background thread that drains samples off the sensorservice event queue and
//! `bus_emit`s `Event::SensorReading`, plus a `set_sensor` entry the effect
//! executor calls to apply the pure module's `Effect::SetSensor` (enable /
//! disable on demand — the battery contract).
//!
//! The binder mechanism lives in the shared `wandr-sensors-client` crate (also used
//! by wandr-host); this file owns only the arbiter-side wiring: the kind↔handle
//! maps (built once from `enumerate`), the descriptor seed into the core Store
//! (so the sensors module derives the proximity threshold from real hardware),
//! and the poll→`bus_emit` loop.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use wandr_arbiter_core::SensorKind;
use wandr_sensors_client::SensorDesc;

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
        27 => Some(SensorKind::DeviceOrientation), // HAL-fused screen rotation
        _ => None,
    }
}

/// Sampling rate for the always-on device-orientation sensor. It's on-change
/// (the rate only bounds latency, not power), so a low rate is plenty.
const ORIENTATION_RATE_HZ: u32 = 5;

/// `kind → SensorDesc` for every recognized sensor, built once at `spawn`.
fn sensors() -> &'static HashMap<SensorKind, SensorDesc> {
    static MAP: OnceLock<HashMap<SensorKind, SensorDesc>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        for s in wandr_sensors_client::enumerate() {
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

/// Channel to the sensor-enable worker thread (set in [`spawn`]). `set_sensor`
/// hands the enable/disable off here so the **slow HAL call runs off the arbiter
/// lock** — see [`set_sensor`].
fn enable_tx() -> &'static Mutex<Option<Sender<(SensorKind, bool, u32)>>> {
    static TX: OnceLock<Mutex<Option<Sender<(SensorKind, bool, u32)>>>> = OnceLock::new();
    TX.get_or_init(|| Mutex::new(None))
}

/// Apply the sensors module's enable/disable decision to the HAL — NON-BLOCKING.
/// Called from `execute_effects`, which holds `arbiter_lock`; the underlying HAL
/// `enableSensor`/`disableSensor` on the qcom SSC can block ~5 s (the SLPI
/// activating an on-change sensor like the ALS), and `reconcile_light` toggles the
/// ALS on EVERY screen on/off — so doing it inline froze the whole arbiter
/// (keyguard / input / screen-power) for ~5 s on every power-press and proximity
/// change (device-observed: "panel ON" → "enabled light" 5 s later, with a host
/// "Broken pipe" from the stall). So we just queue the request to a dedicated
/// worker thread (ordered FIFO, so rapid on/off applies correctly) and return
/// immediately, releasing the lock. Auto-brightness lagging a few seconds is
/// harmless; freezing input is not.
pub fn set_sensor(kind: SensorKind, on: bool, rate_hz: u32) {
    let tx = enable_tx().lock().unwrap_or_else(|e| e.into_inner());
    match tx.as_ref() {
        Some(tx) => {
            let _ = tx.send((kind, on, rate_hz));
        }
        // Worker not spawned yet (shouldn't happen — spawn() sets it before any
        // effect runs). Fall back to applying inline so nothing is silently dropped.
        None => apply_set_sensor(kind, on, rate_hz),
    }
}

/// The actual (potentially slow) HAL toggle — runs on the enable-worker thread,
/// never holding `arbiter_lock`. Looks up the handle for `kind` and toggles it;
/// updates the enabled count that gates the poll loop.
fn apply_set_sensor(kind: SensorKind, on: bool, rate_hz: u32) {
    let Some(sensor) = sensors().get(&kind) else {
        log::warn!("sensor_driver: SetSensor {} but no such sensor on this device", kind.as_wire());
        return;
    };
    if on {
        if wandr_sensors_client::enable(sensor.handle, rate_hz) {
            ENABLED.fetch_add(1, Ordering::Relaxed);
            log::info!("sensor_driver: enabled {} (handle {})", kind.as_wire(), sensor.handle);
        }
    } else {
        wandr_sensors_client::disable(sensor.handle);
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

    // Enable-worker thread: drains queued SetSensor requests and performs the slow
    // HAL enable/disable OFF the arbiter lock (see `set_sensor`). Spawned BEFORE the
    // first `set_sensor` below so that enable is queued, not applied inline.
    let (tx, rx) = channel::<(SensorKind, bool, u32)>();
    *enable_tx().lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    std::thread::Builder::new()
        .name("arbiter-sensor-enable".into())
        .spawn(move || {
            for (kind, on, rate_hz) in rx {
                apply_set_sensor(kind, on, rate_hz);
            }
        })
        .expect("spawn sensor-enable worker thread");

    // Device-orientation is enabled ALWAYS-ON (not ref-counted like proximity/
    // light): auto-rotation must keep working with no surface "holding" it. This
    // is the native replacement for the old `wandr-sensors` daemon's HAL poll —
    // the WM turns each `SensorReading { DeviceOrientation }` into a rotation.
    // (Harmless no-op + warning if the device doesn't expose type 27.)
    if sensors().contains_key(&SensorKind::DeviceOrientation) {
        set_sensor(SensorKind::DeviceOrientation, true, ORIENTATION_RATE_HZ);
    }

    // NOTE: the ambient-light sensor (auto-brightness) is NOT enabled here. Keeping
    // it always-on cost ~5% CPU even with the screen off (the ALS keeps the sensor
    // coprocessor sampling) — device-measured. Instead the power module enables it
    // only while auto-brightness can act (screen on, not blanked, not manual) and
    // disables it when the screen sleeps — see `wandr-arbiter-power::reconcile_light`.

    std::thread::Builder::new()
        .name("arbiter-sensor-driver".into())
        .spawn(|| loop {
            if ENABLED.load(Ordering::Relaxed) == 0 {
                std::thread::sleep(IDLE_INTERVAL);
                continue;
            }
            std::thread::sleep(POLL_INTERVAL);
            // C3 — re-validate the sensor connection each tick (cheap ping when
            // healthy). If wandr-sensormanager/sensorservice restarted, this recreates
            // the event queue and replays our always-on enables (orientation, etc.),
            // so sensors recover without a full stack re-bringup.
            wandr_sensors_client::ensure_connected();
            let samples = wandr_sensors_client::drain_samples();
            if samples.is_empty() {
                continue;
            }
            let map = handle_to_kind();
            let _guard = crate::arbiter_lock().lock().unwrap_or_else(|e| e.into_inner());
            for s in samples {
                let Some(kind) = map.get(&s.handle).copied() else { continue };
                crate::bus_emit(wandr_arbiter_core::Event::SensorReading {
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
