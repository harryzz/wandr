//! wandr-arbiter-sensors — the SensorService role (Arbiter Inc., task 77).
//!
//! The arbiter's single owner of hardware sensors. Built **before** any sensor
//! consumer (proximity-screen-off during calls, accel→orientation,
//! light→auto-brightness) so each consumer plugs in by **reacting to an event**,
//! never by reading the HAL itself. Two responsibilities, both pure:
//!
//!   1. **Enable-on-demand arbitration** (the battery contract). Consumers don't
//!      touch the HAL; they emit [`Event::SensorAcquire`] / [`Event::SensorRelease`]
//!      (the *consumer protocol*). This module ref-counts holders per
//!      [`SensorKind`] and requests [`Effect::SetSensor`]`{on:true}` on the first
//!      holder, `{on:false}` on the last — so a sensor only draws power while
//!      someone needs it.
//!   2. **Raw → semantic translation.** The binary's sensor-driver thread
//!      bus-emits raw [`Event::SensorReading`]s; this module caches them in the
//!      [`Store`] and turns proximity into a debounced [`Event::ProximityChanged`]
//!      with a near/far threshold **derived from the sensor's own `max_range`**
//!      (no hardcoded distance — [[feedback_no_hardcoding]]).
//!
//! The HAL mechanism (rsbinder `ISensorManager`) lives in the binary +
//! `wandr-sensors-client`; this module is desktop-testable via the `report-sensor`
//! sim verb (mirrors `report-orientation`) with no device.

use std::collections::HashMap;
use wandr_arbiter_core::{ArbiterModule, Ctx, Effect, Event, Reply, SensorKind, SensorSlot};

/// "Near" is closer than this fraction of the proximity sensor's `max_range`.
/// The ONE named source of truth for the proximity classification policy (the
/// no-hardcoding rule: a genuinely-needed constant lives in the layer that owns
/// the policy — here). Dimensionless, so it scales with whatever distance range
/// the hardware reports (a binary 0/maxRange sensor and a continuous-cm sensor
/// both classify correctly). AOSP's screen-off logic uses the same shape
/// (`distance < min(5cm, maxRange)`); the midpoint is the device-independent form.
const PROXIMITY_NEAR_FRACTION: f32 = 0.5;

/// Hysteresis half-band, as a fraction of the proximity sensor's `max_range`.
/// A reading hovering near the midpoint must move at least this far past it to
/// flip near↔far, so a hand-wave / noise can't flap the screen. Expressed as a
/// fraction of the sensor's own range (not `resolution` — many proximity sensors
/// are binary and report `resolution == max_range`, which would collapse the
/// window), so it stays device-independent. Second named policy constant.
const PROXIMITY_HYST_FRACTION: f32 = 0.1;

/// Requested HAL sample rate when enabling a sensor, Hz. `0` = "no preference".
/// NOTE the device reality: `wandr-sensors-client::period_us` maps `0` to a **1000 ms**
/// sampling period, and this qcom sensor HAL honors that period even for on-change
/// sensors — so `0` is effectively a *slow* 1 Hz cadence, not "report on crossing
/// ASAP". Fine for sensors where latency doesn't matter; NOT for proximity (see
/// [`rate_for`]).
const DEFAULT_RATE_HZ: u32 = 0;

/// Proximity is latency-critical: the screen-off-on-ear blank during a call must
/// feel near-instant, but at [`DEFAULT_RATE_HZ`] (= 1000 ms period, above) the blank
/// lagged ~1 s+ (device-observed). The proximity HW is on-change with `minDelay=0`,
/// so a fast requested period only *bounds* delivery latency — it cannot spam events
/// (one event per actual crossing). 20 Hz (50 ms) is well under human reaction
/// (~100 ms). One named policy constant; the rate the screen-off use case needs.
const PROXIMITY_RATE_HZ: u32 = 20;

/// The HAL sample rate to request for a given kind. Only proximity overrides the
/// default (its blank latency is user-visible); everything else takes the HAL's
/// natural cadence.
fn rate_for(kind: SensorKind) -> u32 {
    match kind {
        SensorKind::Proximity => PROXIMITY_RATE_HZ,
        _ => DEFAULT_RATE_HZ,
    }
}

/// Synthetic requester id for the `sensor-hold` test verb (negative so it can
/// never collide with a real pid).
const MANUAL_REQUESTER: i32 = -1;

#[derive(Default)]
pub struct SensorsModule {
    /// Requester ids holding each kind enabled (ref-count via membership; one
    /// entry per requester so a release / death drops exactly one reference).
    holders: HashMap<SensorKind, Vec<i32>>,
    /// Current debounced proximity state. `None` until the first reading after an
    /// enable (so a re-enable re-classifies fresh rather than assuming far).
    prox_near: Option<bool>,
}

impl SensorsModule {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a reference; enable the HAL on the 0→1 transition.
    fn acquire(&mut self, kind: SensorKind, requester: i32, ctx: &mut Ctx) {
        let holders = self.holders.entry(kind).or_default();
        let was_empty = holders.is_empty();
        if !holders.contains(&requester) {
            holders.push(requester);
        }
        if was_empty {
            log::info!("sensors: {} enabled (first holder {requester})", kind.as_wire());
            ctx.request(Effect::SetSensor { kind, on: true, rate_hz: rate_for(kind) });
        }
    }

    /// Drop a reference; disable the HAL on the 1→0 transition.
    fn release(&mut self, kind: SensorKind, requester: i32, ctx: &mut Ctx) {
        let Some(holders) = self.holders.get_mut(&kind) else { return };
        let before = holders.len();
        holders.retain(|&r| r != requester);
        if holders.is_empty() && before > 0 {
            log::info!("sensors: {} disabled (last holder {requester} released)", kind.as_wire());
            ctx.request(Effect::SetSensor { kind, on: false, rate_hz: rate_for(kind) });
            // Forget the semantic state so the next enable re-classifies from the
            // first fresh reading rather than emitting a stale crossing.
            if kind == SensorKind::Proximity {
                self.prox_near = None;
            }
        }
    }

    /// Drop every reference a (dead) requester held across all kinds — the
    /// defensive cleanup when a holder's process exits without releasing.
    fn release_all(&mut self, requester: i32, ctx: &mut Ctx) {
        let kinds: Vec<SensorKind> = self
            .holders
            .iter()
            .filter(|(_, hs)| hs.contains(&requester))
            .map(|(k, _)| *k)
            .collect();
        for k in kinds {
            self.release(k, requester, ctx);
        }
    }

    /// Cache a raw reading in the Store and run any semantic translation.
    fn on_reading(&mut self, kind: SensorKind, x: f32, y: f32, z: f32, ts_ns: u64, ctx: &mut Ctx) {
        {
            let slot = ctx.store.sensor_mut(kind);
            slot.last_x = x;
            slot.last_y = y;
            slot.last_z = z;
            slot.last_ts_ns = ts_ns;
            slot.has_reading = true;
        }
        if kind == SensorKind::Proximity {
            self.translate_proximity(x, ctx);
        }
    }

    /// Proximity scalar (`x`, in the sensor's distance unit) → debounced
    /// near/far. Threshold + hysteresis dead-band are derived from the sensor's
    /// `max_range` / `resolution` descriptor (seeded by the driver at enumerate),
    /// so nothing here is device-specific. Emits [`Event::ProximityChanged`] only
    /// on an actual debounced transition.
    fn translate_proximity(&mut self, x: f32, ctx: &mut Ctx) {
        let slot: SensorSlot = match ctx.store.sensor(SensorKind::Proximity) {
            Some(s) if s.max_range > 0.0 => *s,
            _ => {
                // No descriptor yet (driver hasn't enumerated) → we can't honestly
                // classify without inventing a threshold. Skip rather than guess.
                log::debug!("sensors: proximity reading {x} ignored — no descriptor yet");
                return;
            }
        };
        let mid = slot.max_range * PROXIMITY_NEAR_FRACTION;
        // Hysteresis half-band, proportional to the sensor's range (see the const)
        // so a reading hovering at the midpoint can't flap near/far.
        let band = slot.max_range * PROXIMITY_HYST_FRACTION;
        let near = match self.prox_near {
            None => x < mid,            // first reading: classify at the midpoint
            Some(true) => x < mid + band,  // sticky-near: only go far if clearly far
            Some(false) => x < mid - band, // sticky-far: only go near if clearly near
        };
        if self.prox_near != Some(near) {
            self.prox_near = Some(near);
            log::info!("sensors: proximity {} (x={x}, mid={mid})", if near { "near" } else { "far" });
            ctx.emit(Event::ProximityChanged { near });
        }
    }

    // ── Verbs ───────────────────────────────────────────────────────────────

    /// `report-sensor <kind> <x> [y z]` — inject a reading without the HAL
    /// (desktop/pre-device testing; mirrors `report-orientation`). Routes through
    /// the same [`Event::SensorReading`] path the real driver uses.
    fn cmd_report_sensor(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let toks: Vec<&str> = args.split_whitespace().collect();
        let Some(kind_tok) = toks.first() else {
            return Reply::err("report-sensor-usage <kind> <x> [y z]");
        };
        let Some(kind) = SensorKind::from_wire(kind_tok) else {
            return Reply::err(format!("report-sensor-unknown-kind {kind_tok:?}"));
        };
        let parse = |i: usize| toks.get(i).and_then(|t| t.parse::<f32>().ok()).unwrap_or(0.0);
        let Some(_) = toks.get(1) else {
            return Reply::err("report-sensor-missing-x");
        };
        let (x, y, z) = (parse(1), parse(2), parse(3));
        ctx.emit(Event::SensorReading { kind, x, y, z, ts_ns: 0 });
        Reply::ok(format!("report-sensor {} x={x} y={y} z={z}", kind.as_wire()))
    }

    /// `report-sensor-descriptor <kind> <max_range> <resolution>` — set a sensor's
    /// descriptor from an EXTERNAL driver. The in-process `sensor_driver` seeds this
    /// at enumerate via the framework SensorManager, but that's dead under ART-off;
    /// the standalone `wandr-sensors` daemon (which reads the HAL directly) sends it
    /// here so the proximity near/far classifier has the `max_range` it needs.
    fn cmd_report_sensor_descriptor(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let toks: Vec<&str> = args.split_whitespace().collect();
        let (Some(kind_tok), Some(mr_tok)) = (toks.first(), toks.get(1)) else {
            return Reply::err("report-sensor-descriptor-usage <kind> <max_range> <resolution>");
        };
        let Some(kind) = SensorKind::from_wire(kind_tok) else {
            return Reply::err(format!("report-sensor-descriptor-unknown-kind {kind_tok:?}"));
        };
        let Ok(max_range) = mr_tok.parse::<f32>() else {
            return Reply::err(format!("report-sensor-descriptor-bad-max-range {mr_tok:?}"));
        };
        let resolution = toks.get(2).and_then(|t| t.parse::<f32>().ok()).unwrap_or(0.0);
        ctx.store.set_sensor_descriptor(kind, max_range, resolution);
        Reply::ok(format!(
            "report-sensor-descriptor {} max_range={max_range} resolution={resolution}",
            kind.as_wire()
        ))
    }

    /// `sensor-hold <kind> <on|off>` — manually acquire/release a sensor under a
    /// synthetic requester, for device verification of the real HAL enable path
    /// (cover/uncover, battery power-down) without needing a live call to drive
    /// `CommsActive`. Mirrors the `report-*` sim verbs.
    fn cmd_sensor_hold(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let toks: Vec<&str> = args.split_whitespace().collect();
        let Some(kind_tok) = toks.first() else {
            return Reply::err("sensor-hold-usage <kind> <on|off>");
        };
        let Some(kind) = SensorKind::from_wire(kind_tok) else {
            return Reply::err(format!("sensor-hold-unknown-kind {kind_tok:?}"));
        };
        let on = match toks.get(1).copied() {
            Some("on") | Some("1") | Some("true") => true,
            Some("off") | Some("0") | Some("false") => false,
            _ => return Reply::err("sensor-hold-usage <kind> <on|off>"),
        };
        if on {
            self.acquire(kind, MANUAL_REQUESTER, ctx);
        } else {
            self.release(kind, MANUAL_REQUESTER, ctx);
        }
        Reply::ok(format!("sensor-hold {} {}", kind.as_wire(), if on { "on" } else { "off" }))
    }

    /// `sensor-state` — dump ref-count holders + last readings (debug/verify).
    fn cmd_sensor_state(&self, ctx: &mut Ctx) -> Reply {
        let mut parts: Vec<String> = Vec::new();
        for kind in [SensorKind::Proximity, SensorKind::Accelerometer, SensorKind::Light] {
            let holders = self.holders.get(&kind).map(|h| h.len()).unwrap_or(0);
            let slot = ctx.store.sensor(kind);
            let val = match slot {
                Some(s) if s.has_reading => format!("x={:.3} max={:.3}", s.last_x, s.max_range),
                Some(s) => format!("max={:.3} (no reading)", s.max_range),
                None => "unknown".to_string(),
            };
            parts.push(format!("{}[holders={holders} {val}]", kind.as_wire()));
        }
        let near = match self.prox_near {
            Some(true) => "near",
            Some(false) => "far",
            None => "unknown",
        };
        Reply::ok(format!("sensors prox={near} {}", parts.join(" ")))
    }
}

impl ArbiterModule for SensorsModule {
    fn verbs(&self) -> &[&'static str] {
        &["report-sensor", "report-sensor-descriptor", "sensor-state", "sensor-hold"]
    }

    fn on_command(&mut self, verb: &str, args: &str, ctx: &mut Ctx) -> Reply {
        match verb {
            "report-sensor" => self.cmd_report_sensor(args, ctx),
            "report-sensor-descriptor" => self.cmd_report_sensor_descriptor(args, ctx),
            "sensor-state" => self.cmd_sensor_state(ctx),
            "sensor-hold" => self.cmd_sensor_hold(args, ctx),
            other => Reply::err(format!("sensors-unknown-verb {other}")),
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut Ctx) {
        match ev {
            Event::SensorAcquire { kind, requester } => self.acquire(*kind, *requester, ctx),
            Event::SensorRelease { kind, requester } => self.release(*kind, *requester, ctx),
            Event::SensorReading { kind, x, y, z, ts_ns } => {
                self.on_reading(*kind, *x, *y, *z, *ts_ns, ctx)
            }
            // A holder's process exited without releasing — drop its references so
            // the sensor powers down if it was the last.
            Event::SurfaceRemoved { pid } => self.release_all(*pid, ctx),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wandr_arbiter_core::{Registry, Store};

    /// A proximity sensor like the Pixel 2 XL's: binary-ish, maxRange 5, res 5.
    fn seed_proximity(store: &mut Store) {
        store.set_sensor_descriptor(SensorKind::Proximity, 5.0, 5.0);
    }

    fn reg() -> Registry {
        let mut r = Registry::new();
        r.register(Box::new(SensorsModule::new()));
        r
    }

    /// Pull every `SetSensor` effect out of a result batch.
    fn set_sensor_effects(effects: &[Effect]) -> Vec<(SensorKind, bool)> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::SetSensor { kind, on, .. } => Some((*kind, *on)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn report_sensor_emits_proximity_near_then_far() {
        let mut r = reg();
        let mut store = Store::new();
        seed_proximity(&mut store);

        // x=0 (< mid 2.5) → near.
        let (reply, _eff) = r.dispatch_command("report-sensor", "proximity 0", &mut store).unwrap();
        assert!(matches!(reply, Reply::Ok(_)));
        assert_eq!(store.sensor(SensorKind::Proximity).unwrap().last_x, 0.0);

        // x=5 (>= mid) → far. (The ProximityChanged events are internal to the
        // cascade; we assert via the module's reported state.)
        r.dispatch_command("report-sensor", "proximity 5", &mut store).unwrap();
        let (reply, _) = r.dispatch_command("sensor-state", "", &mut store).unwrap();
        match reply {
            Reply::Ok(s) => assert!(s.contains("prox=far"), "expected far, got {s:?}"),
            Reply::Err(e) => panic!("sensor-state err {e}"),
        }
    }

    #[test]
    fn report_sensor_descriptor_enables_classification() {
        // Without a descriptor the classifier can't decide; the new verb supplies it
        // (the ART-off path, where wandr-sensors feeds max_range from the HAL).
        let mut r = reg();
        let mut store = Store::new();
        // No descriptor yet → a reading can't flip prox state (stays unknown).
        r.dispatch_command("report-sensor", "proximity 0", &mut store).unwrap();
        let (reply, _) = r.dispatch_command("sensor-state", "", &mut store).unwrap();
        assert!(matches!(reply, Reply::Ok(ref s) if s.contains("prox=unknown")));
        // Supply the descriptor (max_range 5), then the same reading classifies near.
        let (reply, _) = r
            .dispatch_command("report-sensor-descriptor", "proximity 5 0.1", &mut store)
            .unwrap();
        assert!(matches!(reply, Reply::Ok(_)));
        assert_eq!(store.sensor(SensorKind::Proximity).unwrap().max_range, 5.0);
        r.dispatch_command("report-sensor", "proximity 0", &mut store).unwrap();
        let (reply, _) = r.dispatch_command("sensor-state", "", &mut store).unwrap();
        assert!(matches!(reply, Reply::Ok(ref s) if s.contains("prox=near")), "{reply:?}");
    }

    #[test]
    fn proximity_change_event_only_on_transition() {
        // Drive the module via injected readings and count ProximityChanged via a
        // sink module that records them.
        use std::sync::{Arc, Mutex};
        struct Sink {
            changes: Arc<Mutex<Vec<bool>>>,
        }
        impl ArbiterModule for Sink {
            fn verbs(&self) -> &[&'static str] {
                &[]
            }
            fn on_command(&mut self, _v: &str, _a: &str, _c: &mut Ctx) -> Reply {
                Reply::ok("")
            }
            fn on_event(&mut self, ev: &Event, _c: &mut Ctx) {
                if let Event::ProximityChanged { near } = ev {
                    self.changes.lock().unwrap().push(*near);
                }
            }
        }
        let changes = Arc::new(Mutex::new(Vec::new()));
        let mut r = Registry::new();
        r.register(Box::new(SensorsModule::new()));
        r.register(Box::new(Sink { changes: changes.clone() }));
        let mut store = Store::new();
        seed_proximity(&mut store);

        // near, near (no second event), far, far (no second event), near.
        for x in [0.0_f32, 0.0, 5.0, 5.0, 0.0] {
            r.dispatch_command("report-sensor", &format!("proximity {x}"), &mut store).unwrap();
        }
        assert_eq!(*changes.lock().unwrap(), vec![true, false, true], "one event per transition only");
    }

    #[test]
    fn refcount_enable_on_first_disable_on_last() {
        let mut r = reg();
        let mut store = Store::new();

        // Two holders acquire proximity: only the first enables.
        let eff = r.dispatch_event(Event::SensorAcquire { kind: SensorKind::Proximity, requester: 10 }, &mut store);
        assert_eq!(set_sensor_effects(&eff), vec![(SensorKind::Proximity, true)]);
        let eff = r.dispatch_event(Event::SensorAcquire { kind: SensorKind::Proximity, requester: 11 }, &mut store);
        assert_eq!(set_sensor_effects(&eff), vec![], "second holder doesn't re-enable");

        // First releases: still held → no disable.
        let eff = r.dispatch_event(Event::SensorRelease { kind: SensorKind::Proximity, requester: 10 }, &mut store);
        assert_eq!(set_sensor_effects(&eff), vec![]);
        // Last releases: disable.
        let eff = r.dispatch_event(Event::SensorRelease { kind: SensorKind::Proximity, requester: 11 }, &mut store);
        assert_eq!(set_sensor_effects(&eff), vec![(SensorKind::Proximity, false)]);
    }

    #[test]
    fn surface_removed_releases_holds() {
        let mut r = reg();
        let mut store = Store::new();
        r.dispatch_event(Event::SensorAcquire { kind: SensorKind::Proximity, requester: 10 }, &mut store);
        // The holder dies without releasing → the sensor powers down.
        let eff = r.dispatch_event(Event::SurfaceRemoved { pid: 10 }, &mut store);
        assert_eq!(set_sensor_effects(&eff), vec![(SensorKind::Proximity, false)]);
    }
}
