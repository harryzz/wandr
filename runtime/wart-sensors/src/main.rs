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

use hal::{SensorHal, WartSensorEvent, TYPE_ACCEL, TYPE_DEVICE_ORIENTATION, TYPE_LIGHT, TYPE_PROXIMITY};
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

/// Push one command line to the arbiter (one-shot connect+write+close, like the
/// host's `report_orientation_to_arbiter`). Best-effort.
fn send_arbiter(line: &str) {
    let sock = arbiter_sock_path();
    match UnixStream::connect(&sock) {
        Ok(mut s) => {
            let _ = writeln!(s, "{line}");
            let _ = s.flush();
            let _ = s.shutdown(std::net::Shutdown::Write);
            log::debug!("wart-sensors: {line} → {sock}");
        }
        Err(e) => log::debug!("wart-sensors: {line:?} → {sock} failed: {e}"),
    }
}

/// Reconnect to the sensors HAL after a transport drop and re-enable the sensors
/// we had on (task 89 Issue 1). Blocks (with backoff) until reconnected — the HAL
/// comes back once the churn settles; the alternative (exiting) silently kills
/// auto-brightness / auto-rotation / proximity with no restart.
fn reconnect(hal: &SensorHal, sensors: &[(i32, i64)]) {
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        // backoff 200ms → 2s cap
        std::thread::sleep(std::time::Duration::from_millis((200 * attempt as u64).min(2000)));
        if hal.reopen().is_err() {
            log::debug!("wart-sensors: HAL reopen attempt {attempt} failed — retrying");
            continue;
        }
        let mut all_ok = true;
        for &(stype, period) in sensors {
            if let Err(rc) = hal.enable(stype, period, true) {
                log::warn!("wart-sensors: re-enable type={stype} after reconnect failed (rc={rc})");
                all_ok = false;
            }
        }
        if all_ok {
            log::info!(
                "wart-sensors: HAL reconnected (attempt {attempt}); {} sensor(s) re-enabled",
                sensors.len()
            );
            return;
        }
        // a re-enable hit a fresh transport error → loop drops the handle, retry
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

    // Proximity (task 78 under ART-off): enable it, push its descriptor (the HAL's
    // max_range/resolution) so the arbiter's classifier can decide near/far, and
    // feed each reading. The arbiter then blanks the screen on near *during a call*
    // (wart-arbiter-power gated on CommsActive). On-demand ref-counting via the
    // arbiter's SetSensor effect is a follow-on; always-on proximity is cheap.
    let have_prox = hal.enable(TYPE_PROXIMITY, ROTATION_PERIOD_NS, true).is_ok();
    if have_prox {
        let mr = hal.max_range(TYPE_PROXIMITY);
        let res = hal.resolution(TYPE_PROXIMITY);
        send_arbiter(&format!("report-sensor-descriptor proximity {mr} {res}"));
        log::info!("wart-sensors: proximity enabled (max_range={mr} resolution={res}) → arbiter near/far");
    }
    let mut last_prox = f32::NAN;

    // Ambient light (task 86 — ART-off auto-brightness): enable it, push its
    // descriptor (the HAL's max_range = lux ceiling, which the arbiter's curve uses
    // as its normalization ceiling — no hardcoded lux), and feed each reading. The
    // arbiter's power module maps lux → backlight (curve + smoothing/hysteresis).
    // Mirrors proximity; always-on (cheap; on-demand ref-count is a follow-on).
    let have_light = hal.enable(TYPE_LIGHT, ROTATION_PERIOD_NS, true).is_ok();
    if have_light {
        let mr = hal.max_range(TYPE_LIGHT);
        let res = hal.resolution(TYPE_LIGHT);
        send_arbiter(&format!("report-sensor-descriptor light {mr} {res}"));
        log::info!("wart-sensors: light enabled (max_range={mr} resolution={res}) → arbiter auto-brightness");
    }
    let mut last_lux = f32::NAN;

    // Record the sensors we actually enabled so we can re-enable them after a HAL
    // reconnect (task 89 Issue 1 — the HAL forgets activations when its connection
    // drops). Orientation is whichever of DEVICE_ORIENTATION / ACCEL we enabled above.
    let mut enabled_sensors: Vec<(i32, i64)> = Vec::new();
    enabled_sensors.push((
        if use_hal_orient { TYPE_DEVICE_ORIENTATION } else { TYPE_ACCEL },
        ROTATION_PERIOD_NS,
    ));
    if have_prox {
        enabled_sensors.push((TYPE_PROXIMITY, ROTATION_PERIOD_NS));
    }
    if have_light {
        enabled_sensors.push((TYPE_LIGHT, ROTATION_PERIOD_NS));
    }

    let mut tracker = OrientationTracker::new();
    let mut buf = [WartSensorEvent::default(); 32];
    loop {
        let events = match hal.poll(&mut buf) {
            Ok(ev) => ev,
            Err(rc) => {
                // The sensors HAL connection dropped (DEAD_OBJECT). The shim already
                // nulled its handle (instead of aborting — task 89 Issue 1); reconnect
                // and re-enable, then resume. Force fresh readings so the arbiter
                // immediately gets the current lux/proximity after the gap.
                log::warn!("wart-sensors: HAL poll transport error ({rc}) — reconnecting");
                reconnect(&hal, &enabled_sensors);
                last_prox = f32::NAN;
                last_lux = f32::NAN;
                continue;
            }
        };
        if events.is_empty() {
            // poll blocks; guard against a misbehaving HAL spinning us.
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }
        for ev in events {
            if ev.stype == TYPE_PROXIMITY {
                if ev.x != last_prox {
                    last_prox = ev.x;
                    send_arbiter(&format!("report-sensor proximity {}", ev.x));
                    log::info!("wart-sensors: proximity x={} → arbiter", ev.x);
                }
                continue;
            }
            if ev.stype == TYPE_LIGHT {
                // De-dupe identical consecutive lux (a steady ALS re-reports the
                // same value) so we don't spam the arbiter socket. The arbiter does
                // the smoothing/hysteresis; this is just transport thrift.
                if ev.x != last_lux {
                    last_lux = ev.x;
                    send_arbiter(&format!("report-sensor light {}", ev.x));
                    log::debug!("wart-sensors: light lux={} → arbiter", ev.x);
                }
                continue;
            }
            if use_hal_orient {
                if ev.stype == TYPE_DEVICE_ORIENTATION {
                    // DEVICE_ORIENTATION reports the rotation index in x (0/1/2/3).
                    let rot = ev.x.round().rem_euclid(4.0) as u32;
                    if tracker.current() != Some(rot) {
                        tracker.set(rot); // record (no debounce needed — HAL already debounced)
                        send_arbiter(&format!("report-orientation {rot}"));
                    }
                }
            } else if ev.stype == TYPE_ACCEL {
                if let Some(rot) = tracker.update(ev.x, ev.y, ev.z) {
                    send_arbiter(&format!("report-orientation {rot}"));
                }
            }
        }
    }
}
