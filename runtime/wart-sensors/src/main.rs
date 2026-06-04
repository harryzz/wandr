//! wart-sensors — the ART-off sensor daemon (task 85). Owns the sensor HAL (via the
//! C++ HIDL shim `libwart_sensors_hal.so`), computes screen orientation from the
//! accelerometer in Rust, and pushes `report-orientation <0..3>` to the arbiter so
//! auto-rotation works with the Java framework stopped (SensorService — which
//! normally fuses the `DEVICE_ORIENTATION` sensor — lives in system_server and dies
//! with ART). One process owns the HAL (the HAL's `poll()` is single-consumer);
//! future consumers (proximity → task 78) read from here too.

mod hal;
mod orientation;

use std::io::Write;
use std::os::unix::net::UnixStream;

use hal::{SensorHal, WartSensorEvent, TYPE_ACCEL, TYPE_DEVICE_ORIENTATION};
use orientation::OrientationTracker;

/// Sampling period — ~10 Hz is plenty for rotation + cheap on power. (DEVICE_ORIENTATION
/// is on-change, so its period only bounds latency; the accel fallback samples at it.)
const ROTATION_PERIOD_NS: i64 = 100_000_000;

fn arbiter_sock_path() -> String {
    std::env::var("WART_ARBITER_SOCK")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/data/local/tmp/wart-arbiter.sock".to_string())
}

/// Push `report-orientation <rot>` to the arbiter (one-shot connect+write, like the
/// host's `report_orientation_to_arbiter`). Best-effort.
fn report_orientation(rot: u32) {
    let sock = arbiter_sock_path();
    match UnixStream::connect(&sock) {
        Ok(mut s) => {
            let _ = write!(s, "report-orientation {rot}\n");
            let _ = s.flush();
            let _ = s.shutdown(std::net::Shutdown::Write);
            log::info!("wart-sensors: report-orientation {rot} → {sock}");
        }
        Err(e) => log::debug!("wart-sensors: report-orientation {rot} → {sock} failed: {e}"),
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let hal = match SensorHal::load() {
        Ok(h) => h,
        Err(e) => {
            log::error!("wart-sensors: HAL load failed: {e}");
            std::process::exit(1);
        }
    };
    // Prefer the HAL's fused DEVICE_ORIENTATION (0/1/2/3 in event x) — it's the
    // vendor-tuned rotation sensor (computed on the SSC sensor core on this device),
    // so no accel fusion is needed. Fall back to the accelerometer + our own
    // gravity-vector calc only if the HAL doesn't expose type 27.
    let use_hal_orient = hal
        .enable(TYPE_DEVICE_ORIENTATION, ROTATION_PERIOD_NS, true)
        .is_ok();
    if use_hal_orient {
        log::info!("wart-sensors: DEVICE_ORIENTATION (HAL-fused) → auto-rotation (arbiter {})", arbiter_sock_path());
    } else if hal.enable(TYPE_ACCEL, ROTATION_PERIOD_NS, true).is_ok() {
        log::info!("wart-sensors: no HAL DEVICE_ORIENTATION — accel fallback → auto-rotation");
    } else {
        log::error!("wart-sensors: neither DEVICE_ORIENTATION nor ACCELEROMETER could be enabled");
        std::process::exit(1);
    }

    let mut tracker = OrientationTracker::new();
    let mut buf = [WartSensorEvent::default(); 32];
    loop {
        let events = hal.poll(&mut buf);
        if events.is_empty() {
            // poll blocks; guard against a misbehaving HAL spinning us.
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }
        for ev in events {
            if use_hal_orient {
                if ev.stype == TYPE_DEVICE_ORIENTATION {
                    // DEVICE_ORIENTATION reports the rotation index in x (0/1/2/3).
                    let rot = ev.x.round().rem_euclid(4.0) as u32;
                    if tracker.current() != Some(rot) {
                        tracker.set(rot); // record (no debounce needed — HAL already debounced)
                        report_orientation(rot);
                    }
                }
            } else if ev.stype == TYPE_ACCEL {
                if let Some(rot) = tracker.update(ev.x, ev.y, ev.z) {
                    report_orientation(rot);
                }
            }
        }
    }
}
