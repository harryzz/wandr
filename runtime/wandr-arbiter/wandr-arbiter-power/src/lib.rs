//! wandr-arbiter-power — the PowerManager module (doze policy).
//!
//! Owns the doze POLICY so the arbiter decides and the host applies (the
//! project's central rule, which the v0 host-local doze diverged from). It
//! consumes [`Event::ScreenState`] (the binary's screen poller — On/Vr = live),
//! applies a screen-off **grace**, and on a doze transition fans a
//! `doze <cadence-ms>` line to every tracked host. Each host then slows its own
//! render/bg-tick loop to that cadence (the mechanism), exactly like it applies
//! arbiter-decided geometry/orientation/roles. `cadence=0` = "not dozing".
//!
//! ## Class-based policy
//! The cadence is **per-app**, keyed on the app's power class — the arbiter is
//! the single authority, so it can treat apps differently:
//!   * **background-service** (e.g. Signal): a lenient *maintenance* cadence so it
//!     keeps receiving in a timely way while the screen is off (a Doze exemption).
//!   * everyone else (normal / chrome): a longer *suspend* cadence — nothing to
//!     show off-screen, so back off harder for battery.
//! The class is reported up by the loader (`report-power-class <pid> <class>` —
//! the host parses the manifest `background` flag; "host reads/reports, arbiter
//! owns/decides", like `report-panel`). Unreported pids default to normal.
//!
//! Future home for user-set per-app profiles (restricted/optimized/unrestricted),
//! wakelocks (suppress doze), and maintenance-window alarm batching.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use wandr_arbiter_core::{ArbiterModule, Ctx, Effect, Event, Reply, SensorKind};

/// Screen-off grace before dozing (ms) — the "when to doze" policy.
const DOZE_GRACE_MS: u128 = 60_000;
/// Cadence for a background-service while dozing (ms): keeps receiving.
const DOZE_MAINTENANCE_MS: u64 = 10_000;
/// Cadence for everyone else while dozing (ms): back off harder (off-screen).
const DOZE_SUSPEND_MS: u64 = 60_000;

/// Default screen-off timeout (ms): input-idle this long → sleep the panel + lock,
/// the PowerManagerService role under `--no-art` (no PMS to own
/// `Settings.System.SCREEN_OFF_TIMEOUT`). AOSP's stock default is 30 s; 60 s is a
/// gentler dev default. The ONE named timeout source; live-tunable via
/// `screen-timeout <ms|off>`.
const DEFAULT_SCREEN_OFF_TIMEOUT_MS: u64 = 60_000;

/// How long to keep the ambient-light sensor enabled (warm) after the screen turns
/// off. Re-enabling a *cold* sensor pays a ~5 s platform `enableSensor` timeout (the
/// SLPI powers the sensor down when it has no client, then re-inits it on the next
/// enable — device-measured), which made auto-brightness lag every screen-on. Keeping
/// the ALS warm across brief off periods (a glance / quick lock-unlock) makes the wake
/// instant; only after a longer idle does it go cold (recovering the idle sampling
/// cost). Mirrors Android keeping the ALS active while the display is in use.
const ALS_WARM_GRACE_MS: u64 = 120_000;

// ── Auto-brightness (task 86) ────────────────────────────────────────────────
// Under `--no-art` there is no DisplayManager auto-brightness controller, so the
// power module — already the display-power/backlight authority — maps the ambient
// light sensor to a backlight level. The curve ceiling is the sensor's own
// `max_range` (read from the Store descriptor), so the only fixed numbers here are
// three named, justified policy constants (the no-hardcoding rule).

/// Legibility floor: the smallest backlight fraction auto-brightness will set, so
/// pitch-dark ambient never drives the panel to an unreadable (or off-looking)
/// level. The ONE named brightness-floor policy constant (mirrors
/// `PROXIMITY_NEAR_FRACTION` in the sensors module). ~4% ≈ a dim-but-legible panel
/// in a dark room; a manual override or `SetDisplayPower(off)` can still go lower.
const MIN_FRACTION: f32 = 0.04;

/// Hysteresis dead-band on the target fraction: a new level is only pushed to
/// the panel once it differs from the last applied level by more than this. The ALS
/// is on-change (one event per settled lux), so we map each reading straight to its
/// target and use this dead-band as the anti-flicker filter: a jittery sensor
/// flipping between nearby lux won't write the panel unless the change clears ~3% of
/// full range (above the just-noticeable-difference for backlight).
const MIN_STEP: f32 = 0.03;

/// Ambient lux at (and above) which the panel runs at FULL brightness — the curve's
/// normalization ceiling. This is a *perceptual/display* reference, NOT the sensor's
/// `max_range`: the ALS saturates at ~32767 lux (direct sunlight), but normalizing
/// against that wastes almost the whole backlight range on outdoor brightness you
/// never see indoors, so cover→uncover barely moves. ~600 lux is bright-indoor /
/// overcast-daylight — beyond it more ambient doesn't improve indoor readability, so
/// we clamp to full. The ONE named curve-ceiling policy constant; the default for the
/// live-tunable `brightness-scale` verb (lower = reaches full sooner / steeper).
const FULL_SCALE_LUX: f32 = 600.0;

/// Sample rate (Hz) requested for the ambient-light sensor while it's enabled.
/// The ALS is on-change and ambient light changes slowly, and the curve has its own
/// `MIN_STEP` hysteresis, so a low rate is plenty — and it's only enabled while the
/// screen is on ([`PowerModule::reconcile_light`]). One named policy constant.
const LIGHT_RATE_HZ: u32 = 5;

#[derive(Default)]
pub struct PowerModule {
    /// When the screen last went non-live (`None` = live). Transient module state.
    screen_off_at: Option<Instant>,
    dozing: bool,
    /// Pids the loader reported as background-services (get the maintenance
    /// cadence). Everyone else is normal. Cleaned on `SurfaceRemoved`.
    bg_service: HashSet<i32>,
    /// M3b — pids in an active comms session (a call). They are NEVER dozed
    /// (`cadence 0`) so a call keeps running with the screen off. Driven by
    /// `Event::CommsActive` from the audio module; cleaned on `SurfaceRemoved`.
    comms: HashSet<i32>,
    /// Task 78 — whether we have forced the panel OFF for proximity (phone at the
    /// ear during a call). Tracked so we toggle on transitions only and, crucially,
    /// **always restore** the panel when the call ends / the sensor uncovers (a
    /// stuck-off panel would feel like a bricked device).
    blanked: bool,
    /// Task 81 — the user-facing panel power (POWER-key toggle). With ART off the
    /// arbiter is the sole display-power authority (no PMS), so this is the
    /// source of truth that a proximity uncover restores TO (not unconditionally
    /// on). Starts on; the binary force-ons the panel at boot under `WANDR_NO_ART`.
    panel_on: bool,

    // ── Auto-brightness state (task 86) ──────────────────────────────────────
    /// EMA-smoothed ambient brightness fraction (0.0–1.0). `None` until the first
    /// light reading; persists across blanks/power so a wake restores the
    /// ambient-correct level rather than the boot default.
    light_frac: Option<f32>,
    /// Manual brightness override (`Some` disables auto until `brightness auto`).
    /// The hook for a future settings/brightness-slider; for now the `brightness`
    /// verb sets it.
    manual_frac: Option<f32>,
    /// Last fraction actually pushed to the panel — the hysteresis reference
    /// (`MIN_STEP`) so we only re-apply on a perceptible change.
    last_applied_frac: Option<f32>,
    /// Curve ceiling: ambient lux at which the panel hits full brightness
    /// ([`FULL_SCALE_LUX`] default). Live-tunable via `brightness-scale <lux>` so the
    /// dark→bright spread can be dialed in without a rebuild.
    full_scale_lux: f32,
    /// Last raw lux reading — kept so a live `brightness-scale` change can recompute
    /// the level immediately from the same ambient (snap, no wait for the next read).
    last_lux: Option<f32>,

    // ── Screen-off timeout state (PowerManager role, task 86 follow-on) ───────
    /// When user input last happened (the dispatcher pokes `user-activity`). `None`
    /// until the first poke / tick; the inactivity check seeds it to "now" so the
    /// countdown starts. Reset on wake so a wake doesn't immediately re-sleep.
    last_activity: Option<Instant>,
    /// Screen-off timeout (`None` = never auto-sleep). Live-tunable via
    /// `screen-timeout <ms|off>`; defaults to [`DEFAULT_SCREEN_OFF_TIMEOUT_MS`].
    screen_off_timeout: Option<Duration>,
    /// Whether we currently hold the ambient-light sensor enabled (task 86 / CPU
    /// fix). The light sensor is enabled ONLY while auto-brightness can act on it
    /// (see [`reconcile_light`]); leaving it always-on kept the sensor coprocessor
    /// sampling the ALS even with the screen off, costing ~5% CPU at idle.
    light_on: bool,
}

/// Pure doze decision: given how long the screen has been off (`off_ms`, `None`
/// = live) and the current dozing flag, return `Some(new)` on a transition else
/// `None`. Split out so the grace boundary is unit-testable without real time.
fn decide(off_ms: Option<u128>, dozing: bool) -> Option<bool> {
    let dozing_now = off_ms.map(|ms| ms >= DOZE_GRACE_MS).unwrap_or(false);
    (dozing_now != dozing).then_some(dozing_now)
}

/// Pure ambient-light → backlight curve (task 86). Perceived brightness is roughly
/// logarithmic in illuminance, so map `lux` through `log10` normalized by
/// `full_scale_lux` — the ambient at which the panel reaches full brightness (see
/// [`FULL_SCALE_LUX`]; a perceptual reference, NOT the sensor's saturation range, so
/// the indoor lux band spans the whole backlight). Clamped to `[MIN_FRACTION, 1.0]`.
/// Split out so the curve is unit-testable without a device.
fn lux_to_fraction(lux: f32, full_scale_lux: f32) -> f32 {
    // Guard a degenerate ceiling (≤ 0 → floor).
    if full_scale_lux <= 0.0 {
        return MIN_FRACTION;
    }
    let frac = (lux.max(0.0) + 1.0).log10() / (full_scale_lux + 1.0).log10();
    frac.clamp(MIN_FRACTION, 1.0)
}

impl PowerModule {
    pub fn new() -> Self {
        Self {
            panel_on: true, // the screen comes up on
            full_scale_lux: FULL_SCALE_LUX,
            screen_off_timeout: Some(Duration::from_millis(DEFAULT_SCREEN_OFF_TIMEOUT_MS)),
            ..Self::default()
        }
    }

    /// Task 81 — set the user-facing panel power and broadcast it: drive
    /// SurfaceFlinger (the applier executes the effect via wandr-screen under
    /// ART-off) AND emit [`Event::ScreenState`] so the rest of the system reacts
    /// exactly as it does to a real power transition — doze grace begins on off,
    /// keyguard auto-locks on off, both clear on on. A proximity blank is an
    /// independent transient override on top of this (uncover restores TO it).
    fn set_panel_on(&mut self, on: bool, ctx: &mut Ctx) {
        self.panel_on = on;
        // If proximity has the panel blanked, don't fight it on the hardware —
        // the blank wins until uncover, which will then restore to this `panel_on`.
        if !self.blanked {
            ctx.request(Effect::SetDisplayPower { on });
        }
        // Task 86 — waking the panel restores the ambient/manual backlight level
        // (the `SetDisplayPower(on)` applier only sets the boot default), so the
        // screen comes back at the right brightness, not the default. Also restart
        // the inactivity countdown so a wake doesn't immediately re-sleep.
        if on {
            self.reassert_backlight(ctx);
            self.last_activity = Some(Instant::now());
        }
        ctx.emit(Event::ScreenState { live: on });
        // Enable/disable the ALS immediately on a panel transition (don't wait for the
        // next idle tick) so auto-brightness is live the moment the screen wakes.
        self.reconcile_light(ctx);
        log::info!("arbiter: panel {} (power-key/explicit)", if on { "ON" } else { "OFF" });
    }

    /// `power-key [pid]` — a hardware POWER press (the host forwards it; pid is
    /// informational). Toggles the panel, the ART-off analogue of PMS's
    /// power-button→screen-toggle.
    fn cmd_power_key(&mut self, _args: &str, ctx: &mut Ctx) -> Reply {
        let on = !self.panel_on;
        self.set_panel_on(on, ctx);
        Reply::ok(format!("panel {}", if on { "on" } else { "off" }))
    }

    /// `panel <on|off>` — explicit panel power (boot force-on / scripting).
    fn cmd_panel(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let on = match args.split_whitespace().next() {
            Some("on") | Some("1") => true,
            Some("off") | Some("0") => false,
            other => return Reply::err(format!("panel-args: expected on|off, got {other:?}")),
        };
        self.set_panel_on(on, ctx);
        Reply::ok(format!("panel {}", if on { "on" } else { "off" }))
    }

    /// The cadence (ms) to send a pid while dozing, by its class. A comms-session
    /// pid is never dozed (cadence 0) — a live call must keep running off-screen.
    fn cadence_for(&self, pid: i32) -> u64 {
        if self.comms.contains(&pid) {
            0 // in a call — never doze
        } else if self.bg_service.contains(&pid) {
            DOZE_MAINTENANCE_MS
        } else {
            DOZE_SUSPEND_MS
        }
    }

    /// Apply the panel-blank state: SurfaceFlinger panel power AND touch
    /// suppression move together (task 79) so they can never drift — a blanked
    /// panel always has touch dropped, and restoring the panel always restores
    /// touch. The suppress flag is fanned to EVERY tracked host (a cheek at the
    /// ear can land on chrome too, and the screen is off so nothing needs touch).
    fn set_panel_blanked(&mut self, blank: bool, ctx: &mut Ctx) {
        // On blank → off; on uncover → restore to the user-facing `panel_on`
        // (task 81), not unconditionally on, so a power-off during a call survives
        // a proximity cycle.
        let on = if blank { false } else { self.panel_on };
        ctx.request(Effect::SetDisplayPower { on });
        for app in ctx.store.apps_snapshot() {
            ctx.deliver_to_host(app.pid, format!("input-suppress {}\n", blank as u8));
        }
        self.blanked = blank;
    }

    /// Fail-safe: restore the panel + touch if proximity had blanked them.
    /// Idempotent — safe to call on every call-end / surface-removed. A stuck-off
    /// panel (or dead touch) would feel like a bricked device, so this is the
    /// invariant the policy guarantees.
    fn ensure_unblanked(&mut self, ctx: &mut Ctx) {
        if self.blanked {
            self.set_panel_blanked(false, ctx);
            // Task 86 — uncover restores the ambient/manual backlight (the panel-on
            // applier only set the boot default).
            self.reassert_backlight(ctx);
            log::info!("arbiter: proximity blank cleared → panel ON + touch resumed");
        }
    }

    fn on_screen_state(&mut self, live: bool, ctx: &mut Ctx) {
        if live {
            self.screen_off_at = None;
        } else if self.screen_off_at.is_none() {
            self.screen_off_at = Some(Instant::now());
        }
        let off_ms = self.screen_off_at.map(|t| t.elapsed().as_millis());
        let Some(new_dozing) = decide(off_ms, self.dozing) else {
            return;
        };
        self.dozing = new_dozing;
        // Fan the per-app cadence to every tracked host (dumb appliers). On EXIT
        // everyone gets `doze 0`; on ENTER each gets its class's cadence. A dead
        // pid's socket just fails silently in the executor.
        for app in ctx.store.apps_snapshot() {
            let cadence = if new_dozing { self.cadence_for(app.pid) } else { 0 };
            ctx.deliver_to_host(app.pid, format!("doze {cadence}\n"));
        }
        log::info!(
            "arbiter: doze {} — fanned per-class to hosts (maintenance={DOZE_MAINTENANCE_MS}ms / suspend={DOZE_SUSPEND_MS}ms, {} bg-services)",
            if new_dozing { "ENTER" } else { "EXIT" },
            self.bg_service.len()
        );
    }

    /// `report-power-class <pid> <bg-service|normal>` — the loader reports an
    /// app's power class at startup (host reads the manifest; arbiter owns it).
    fn cmd_report_class(&mut self, args: &str, _ctx: &mut Ctx) -> Reply {
        let mut t = args.split_whitespace();
        let (Some(pid_s), Some(class)) = (t.next(), t.next()) else {
            return Reply::err("report-power-class-args: expected <pid> <bg-service|normal>");
        };
        let Ok(pid) = pid_s.parse::<i32>() else {
            return Reply::err(format!("report-power-class-bad-pid {pid_s}"));
        };
        match class {
            "bg-service" => {
                self.bg_service.insert(pid);
            }
            _ => {
                self.bg_service.remove(&pid);
            }
        }
        log::info!("arbiter: power-class pid={pid} class={class}");
        Reply::ok(format!("power-class pid={pid} class={class}"))
    }

    // ── Auto-brightness (task 86) ────────────────────────────────────────────

    /// Whether auto-brightness may currently drive the panel: only while the panel
    /// is on, not proximity-blanked, and no manual override is in force. (Brightness
    /// is meaningless on a powered-off/blanked panel, and a manual level wins.)
    fn auto_brightness_active(&self) -> bool {
        self.panel_on && !self.blanked && self.manual_frac.is_none()
    }

    /// Whether the ALS should be ENABLED (kept warm) right now — distinct from
    /// [`auto_brightness_active`] (whether to APPLY brightness). We keep it warm
    /// whenever auto-brightness mode is on (no manual override) and the screen is on
    /// OR was on within [`ALS_WARM_GRACE_MS`] — so a quick lock→unlock finds the
    /// sensor already warm (no ~5 s cold re-enable). Blanked (proximity, mid-call) does
    /// NOT disable it: the panel flips off/on rapidly there and we must not thrash the
    /// sensor. Only a sustained screen-off lets it go cold (recovering idle cost).
    fn light_should_be_warm(&self) -> bool {
        if self.manual_frac.is_some() {
            return false; // manual override: ambient tracking off, no ALS needed
        }
        if self.panel_on {
            return true;
        }
        match self.screen_off_at {
            Some(t) => t.elapsed() < Duration::from_millis(ALS_WARM_GRACE_MS),
            None => true, // screen-off instant not recorded → treat as recently-on
        }
    }

    /// Enable/disable the ambient-light sensor to match [`light_should_be_warm`].
    /// Idempotent — only toggles the HAL on a real change. Called after anything that
    /// changes `panel_on` / `manual_frac` AND every idle tick (so the warm grace
    /// expires the enable once the screen has been off long enough).
    fn reconcile_light(&mut self, ctx: &mut Ctx) {
        let want = self.light_should_be_warm();
        if want != self.light_on {
            ctx.request(Effect::SetSensor {
                kind: SensorKind::Light,
                on: want,
                rate_hz: LIGHT_RATE_HZ,
            });
            self.light_on = want;
            log::info!("arbiter: auto-brightness light sensor {}",
                if want { "ENABLED (warm)" } else { "DISABLED (idle grace elapsed/manual)" });
        }
    }

    /// A new ambient-light reading (`lux`). Maps it straight through the curve to a
    /// target fraction and pushes it if it moved more than `MIN_STEP`. No cross-sample
    /// smoothing: the ALS is on-change (one event per settled lux), so a single
    /// reading must reach its target — an EMA would only step partway and freeze. The
    /// curve ceiling is [`PowerModule::full_scale_lux`] (a perceptual full-brightness
    /// reference, NOT the sensor's saturation range — see [`FULL_SCALE_LUX`]).
    fn on_light(&mut self, lux: f32, ctx: &mut Ctx) {
        self.last_lux = Some(lux);
        self.light_frac = Some(lux_to_fraction(lux, self.full_scale_lux));
        self.apply_auto_brightness(ctx);
    }

    /// Push the smoothed ambient level to the panel, gated by
    /// [`auto_brightness_active`] and the [`MIN_STEP`] hysteresis dead-band (so
    /// sub-perceptible jitter never reaches the hardware). Tracks the applied level
    /// for the next comparison.
    fn apply_auto_brightness(&mut self, ctx: &mut Ctx) {
        if !self.auto_brightness_active() {
            return;
        }
        let Some(frac) = self.light_frac else { return };
        if let Some(last) = self.last_applied_frac {
            if (frac - last).abs() < MIN_STEP {
                return;
            }
        }
        self.last_applied_frac = Some(frac);
        ctx.request(Effect::SetBacklight { level: frac, sensor: true });
    }

    /// Re-assert the correct backlight after a wake / uncover / `brightness auto`,
    /// bypassing the hysteresis dead-band (a one-off, not a stream). Emits the
    /// manual override if set, else the smoothed ambient level; no-op while the
    /// panel is off/blanked (nothing to light). The boot default set by
    /// `apply_display_power` holds until the first ambient reading arrives.
    fn reassert_backlight(&mut self, ctx: &mut Ctx) {
        if !self.panel_on || self.blanked {
            return;
        }
        if let Some(frac) = self.manual_frac.or(self.light_frac) {
            self.last_applied_frac = Some(frac);
            // SENSOR unless a manual override is in force (then it's a USER level).
            ctx.request(Effect::SetBacklight { level: frac, sensor: self.manual_frac.is_none() });
        }
    }

    /// `brightness <auto|0.0..1.0>` — set or clear the manual override (the hook a
    /// future brightness slider drives). `auto` returns to ambient tracking and
    /// re-applies the current ambient level immediately.
    fn cmd_brightness(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        match args.split_whitespace().next() {
            Some("auto") => {
                self.manual_frac = None;
                self.reassert_backlight(ctx);
                log::info!("arbiter: brightness → auto");
                Reply::ok("brightness auto")
            }
            Some(tok) => match tok.parse::<f32>() {
                Ok(f) if (0.0..=1.0).contains(&f) => {
                    self.manual_frac = Some(f);
                    if self.panel_on && !self.blanked {
                        self.last_applied_frac = Some(f);
                        ctx.request(Effect::SetBacklight { level: f, sensor: false });
                    }
                    log::info!("arbiter: brightness → manual {f}");
                    Reply::ok(format!("brightness {f}"))
                }
                _ => Reply::err("brightness-args: expected auto|0.0..1.0"),
            },
            None => Reply::err("brightness-usage: brightness <auto|0.0..1.0>"),
        }
    }

    /// `brightness-scale <lux>` — live-tune the curve ceiling (ambient lux at which
    /// the panel reaches full brightness; [`FULL_SCALE_LUX`] default). Lower =
    /// reaches full sooner / steeper dark→bright spread. Recomputes from the last
    /// ambient reading and re-applies immediately (no wait for the next sample) so
    /// the feel can be dialed in without a rebuild.
    fn cmd_brightness_scale(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let Some(tok) = args.split_whitespace().next() else {
            return Reply::err("brightness-scale-usage: brightness-scale <lux>");
        };
        let Ok(lux) = tok.parse::<f32>() else {
            return Reply::err(format!("brightness-scale-bad-lux {tok:?}"));
        };
        if lux <= 0.0 {
            return Reply::err("brightness-scale: lux must be > 0");
        }
        self.full_scale_lux = lux;
        // Recompute from the last ambient (snap, not EMA) so tuning is immediate.
        if let Some(last) = self.last_lux {
            self.light_frac = Some(lux_to_fraction(last, self.full_scale_lux));
            if self.manual_frac.is_none() {
                self.reassert_backlight(ctx);
            }
        }
        log::info!("arbiter: brightness-scale → {lux} lux full-scale");
        Reply::ok(format!("brightness-scale {lux}"))
    }

    // ── Screen-off timeout (PowerManager role, task 86 follow-on) ─────────────

    /// `user-activity` — the input dispatcher (wandr-inputflinger, the wandr
    /// "PhoneWindowManager") pokes this on real user input, mirroring AOSP's
    /// `InputDispatcher::pokeUserActivity → PowerManagerService.userActivity`. It
    /// just resets the idle clock; it never wakes the panel (only POWER does), so a
    /// stray touch while off can't turn the screen on.
    fn cmd_user_activity(&mut self, _args: &str, _ctx: &mut Ctx) -> Reply {
        self.last_activity = Some(Instant::now());
        Reply::ok("user-activity")
    }

    /// `screen-timeout <ms|off>` — live-set the inactivity screen-off timeout
    /// (`off`/`0` disables auto-sleep). The PowerManager `SCREEN_OFF_TIMEOUT` knob;
    /// resets the countdown so the new value takes effect from now.
    fn cmd_screen_timeout(&mut self, args: &str, _ctx: &mut Ctx) -> Reply {
        match args.split_whitespace().next() {
            Some("off") | Some("none") | Some("0") => {
                self.screen_off_timeout = None;
                log::info!("arbiter: screen-timeout → off (no auto-sleep)");
                Reply::ok("screen-timeout off")
            }
            Some(tok) => match tok.parse::<u64>() {
                Ok(ms) => {
                    self.screen_off_timeout = Some(Duration::from_millis(ms));
                    self.last_activity = Some(Instant::now());
                    log::info!("arbiter: screen-timeout → {ms} ms");
                    Reply::ok(format!("screen-timeout {ms}"))
                }
                Err(_) => Reply::err("screen-timeout-args: expected <ms|off>"),
            },
            None => Reply::err("screen-timeout-usage: screen-timeout <ms|off>"),
        }
    }

    /// Inactivity tick (PowerManager screen-off-timeout). If the panel is on (and
    /// not proximity-blanked) and input has been idle past the timeout, sleep it —
    /// reusing `set_panel_on(false)`, so this cascades exactly like a POWER-off:
    /// keyguard auto-locks, doze grace starts, the panel powers off, backlight → 0.
    /// Pure decision is [`idle_should_sleep`]; this wires it to the clock + state.
    fn on_idle_tick(&mut self, ctx: &mut Ctx) {
        // Reconcile the light-sensor enable to the auto-brightness window every tick
        // (cheap, idempotent). This runs BEFORE the early returns so it also disables
        // the ALS once the screen is off, and enables it at boot (panel starts on).
        self.reconcile_light(ctx);
        if !self.panel_on || self.blanked {
            return;
        }
        // A live call holds the screen awake: never idle-sleep the panel while a
        // comms session is active. This is the no-ART analogue of FLAG_KEEP_SCREEN_ON
        // / a call's screen wakelock, and it lives here (the arbiter is the sole power
        // authority) rather than in the client — the client only signals call
        // start/end via audio-focus → `Event::CommsActive`. Suppressing the idle
        // screen-off also suppresses the keyguard auto-lock (it triggers on a real
        // screen-off), so a video call you're watching without touching never sleeps
        // or locks mid-call. (At-ear proximity blanking is separate — task 78 — and
        // still restores on uncover / call-end.)
        if !self.comms.is_empty() {
            return;
        }
        let Some(timeout) = self.screen_off_timeout else { return };
        let idle = match self.last_activity {
            Some(t) => t.elapsed(),
            None => {
                // First tick with no recorded activity → start the countdown now.
                self.last_activity = Some(Instant::now());
                return;
            }
        };
        if idle_should_sleep(idle, timeout) {
            log::info!("arbiter: screen-off timeout — idle {}s ≥ {}s → panel OFF + lock",
                idle.as_secs(), timeout.as_secs());
            self.set_panel_on(false, ctx);
        }
    }
}

/// Pure screen-off-timeout decision: sleep once input has been idle at least as long
/// as the timeout. Split out so the boundary is unit-testable without real time.
fn idle_should_sleep(idle: Duration, timeout: Duration) -> bool {
    idle >= timeout
}

impl ArbiterModule for PowerModule {
    fn verbs(&self) -> &[&'static str] {
        &["report-power-class", "power-key", "panel", "brightness", "brightness-scale",
          "user-activity", "screen-timeout"]
    }

    fn on_command(&mut self, verb: &str, args: &str, ctx: &mut Ctx) -> Reply {
        match verb {
            "report-power-class" => self.cmd_report_class(args, ctx),
            "power-key" => self.cmd_power_key(args, ctx),
            "panel" => self.cmd_panel(args, ctx),
            "brightness" => self.cmd_brightness(args, ctx),
            "brightness-scale" => self.cmd_brightness_scale(args, ctx),
            "user-activity" => self.cmd_user_activity(args, ctx),
            "screen-timeout" => self.cmd_screen_timeout(args, ctx),
            other => Reply::err(format!("power-unknown-verb {other}")),
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut Ctx) {
        match ev {
            Event::ScreenState { live } => self.on_screen_state(*live, ctx),
            Event::SurfaceRemoved { pid } => {
                self.bg_service.remove(pid);
                self.comms.remove(pid);
                // Task 78 fail-safe: if the call host died while we'd blanked the
                // panel for proximity, restore it (no live call to uncover it).
                if self.comms.is_empty() {
                    self.ensure_unblanked(ctx);
                }
            }
            // M3b — a call started/ended: update the keep-alive set. If we're
            // already dozing, re-fan THIS pid's cadence now (a call starting
            // mid-doze must wake to 0; ending mid-doze re-dozes to its class).
            Event::CommsActive { pid, active } => {
                if *active { self.comms.insert(*pid); } else { self.comms.remove(pid); }
                // Task 78 fail-safe: a call ending must restore the panel if
                // proximity had blanked it (the user can't uncover it post-call).
                if !*active && self.comms.is_empty() {
                    self.ensure_unblanked(ctx);
                }
                // Task 77 — proximity is only worth powering during a call (the
                // screen-off-on-ear use case). Express that intent to the
                // SensorService via the consumer protocol (acquire on call start,
                // release on end); the sensors module ref-counts + drives the HAL.
                // Power never reads the sensor itself — it reacts to
                // `Event::ProximityChanged` below.
                let kind = SensorKind::Proximity;
                if *active {
                    ctx.emit(Event::SensorAcquire { kind, requester: *pid });
                } else {
                    ctx.emit(Event::SensorRelease { kind, requester: *pid });
                }
                if self.dozing {
                    let cadence = self.cadence_for(*pid);
                    ctx.deliver_to_host(*pid, format!("doze {cadence}\n"));
                    log::info!("arbiter: comms {} pid={pid} mid-doze → doze {cadence}",
                        if *active { "start" } else { "end" });
                }
            }
            // Task 78 — proximity screen-off during a call. Only blank while a
            // call is active (phone at the ear); never otherwise. Toggle on the
            // debounced transition and track `blanked` so the fail-safes below can
            // always restore the panel.
            Event::ProximityChanged { near } => {
                let in_call = !self.comms.is_empty();
                if in_call && *near && !self.blanked {
                    self.set_panel_blanked(true, ctx);
                    log::info!("arbiter: proximity near + in-call → panel OFF + touch suppressed");
                } else if !*near && self.blanked {
                    self.ensure_unblanked(ctx);
                } else if !in_call && self.blanked {
                    // Defensive: a call ended between the blank and this event.
                    self.ensure_unblanked(ctx);
                }
            }
            // Task 86 — ambient light → backlight (auto-brightness). The lux is the
            // reading's `x`. Other sensor kinds are owned by their own consumers.
            Event::SensorReading { kind: SensorKind::Light, x, .. } => self.on_light(*x, ctx),
            // PowerManager screen-off-timeout — the inactivity ticker fired.
            Event::IdleTick => self.on_idle_tick(ctx),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Instant, SystemTime};
    use wandr_arbiter_core::{AppState, Effect, Registry, Store};

    #[test]
    fn grace_boundary_decision() {
        assert_eq!(decide(None, false), None);
        assert_eq!(decide(Some(59_000), false), None);
        assert_eq!(decide(Some(60_000), false), Some(true));
        assert_eq!(decide(Some(120_000), true), None);
        assert_eq!(decide(None, true), Some(false));
    }

    fn add_app(s: &mut Store, id: &str, pid: i32) {
        s.insert_app(AppState {
            app_id: id.to_string(),
            pid,
            launched_at: SystemTime::now(),
            launched_mono: Instant::now(),
        });
    }

    fn doze_line(eff: &[Effect], pid: i32) -> Option<String> {
        eff.iter().find_map(|e| match e {
            Effect::HostLine { pid: p, line } if *p == pid => Some(line.trim().to_string()),
            _ => None,
        })
    }

    #[test]
    fn comms_pid_kept_out_of_doze() {
        let mut r = Registry::new();
        let mut m = PowerModule::new();
        m.screen_off_at = Some(Instant::now() - std::time::Duration::from_millis(61_000));
        r.register(Box::new(m));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);
        // A call is active on pid 10 (not yet dozing → no immediate fan).
        r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);
        // Screen off past grace → ENTER doze: the call host gets `doze 0`, not suspend.
        let eff = r.dispatch_event(Event::ScreenState { live: false }, &mut store);
        assert_eq!(doze_line(&eff, 10).as_deref(), Some("doze 0"));
    }

    #[test]
    fn call_start_and_end_mid_doze_refan() {
        let mut r = Registry::new();
        let mut m = PowerModule::new();
        m.screen_off_at = Some(Instant::now() - std::time::Duration::from_millis(61_000));
        r.register(Box::new(m));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);
        // Already dozing (pid 10 = normal → suspend).
        let eff = r.dispatch_event(Event::ScreenState { live: false }, &mut store);
        assert_eq!(doze_line(&eff, 10).as_deref(), Some(&format!("doze {DOZE_SUSPEND_MS}")[..]));
        // Call starts mid-doze → immediate wake to 0.
        let eff = r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);
        assert_eq!(doze_line(&eff, 10).as_deref(), Some("doze 0"));
        // Call ends mid-doze → re-doze to its class cadence.
        let eff = r.dispatch_event(Event::CommsActive { pid: 10, active: false }, &mut store);
        assert_eq!(doze_line(&eff, 10).as_deref(), Some(&format!("doze {DOZE_SUSPEND_MS}")[..]));
    }

    #[test]
    fn doze_fans_per_class_cadence() {
        let mut r = Registry::new();
        let mut m = PowerModule::new();
        m.screen_off_at = Some(Instant::now() - std::time::Duration::from_millis(61_000));
        r.register(Box::new(m));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);
        add_app(&mut store, "game", 20);

        // sig reports bg-service; game stays normal.
        r.dispatch_command("report-power-class", "10 bg-service", &mut store).unwrap();

        // Screen off past grace → ENTER: sig gets maintenance (10s), game suspend (60s).
        let eff = r.dispatch_event(Event::ScreenState { live: false }, &mut store);
        let cad = |pid: i32| {
            eff.iter().find_map(|e| match e {
                Effect::HostLine { pid: p, line } if *p == pid => Some(line.clone()),
                _ => None,
            })
        };
        assert_eq!(cad(10).as_deref(), Some("doze 10000\n")); // bg-service → maintenance
        assert_eq!(cad(20).as_deref(), Some("doze 60000\n")); // normal → suspend

        // Screen on → EXIT: both get doze 0.
        let eff = r.dispatch_event(Event::ScreenState { live: true }, &mut store);
        assert_eq!(
            eff.iter()
                .filter(|e| matches!(e, Effect::HostLine { line, .. } if line == "doze 0\n"))
                .count(),
            2
        );
    }

    #[test]
    fn surface_removed_forgets_class() {
        let mut r = Registry::new();
        let mut m = PowerModule::new();
        m.screen_off_at = Some(Instant::now() - std::time::Duration::from_millis(61_000));
        r.register(Box::new(m));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);
        r.dispatch_command("report-power-class", "10 bg-service", &mut store).unwrap();
        // App dies → its class is forgotten (a recycled pid mustn't inherit it).
        r.dispatch_event(Event::SurfaceRemoved { pid: 10 }, &mut store);
        // ENTER doze: pid 10 now defaults to normal (suspend), not maintenance.
        let eff = r.dispatch_event(Event::ScreenState { live: false }, &mut store);
        let line = eff.iter().find_map(|e| match e {
            Effect::HostLine { pid: 10, line } => Some(line.clone()),
            _ => None,
        });
        assert_eq!(line.as_deref(), Some("doze 60000\n"));
    }

    /// Task 77 — the consumer protocol: a call start/end makes power acquire /
    /// release proximity (so the sensor is only on during calls). Asserted via a
    /// sink module that records the emitted intents (keeps power decoupled from
    /// the sensors crate).
    #[test]
    fn comms_acquires_and_releases_proximity() {
        use std::sync::{Arc, Mutex};
        use wandr_arbiter_core::{Ctx, SensorKind};

        struct Sink {
            seen: Arc<Mutex<Vec<(bool, SensorKind, i32)>>>, // (acquire?, kind, requester)
        }
        impl ArbiterModule for Sink {
            fn verbs(&self) -> &[&'static str] {
                &[]
            }
            fn on_command(&mut self, _v: &str, _a: &str, _c: &mut Ctx) -> Reply {
                Reply::ok("")
            }
            fn on_event(&mut self, ev: &Event, _c: &mut Ctx) {
                match ev {
                    Event::SensorAcquire { kind, requester } => {
                        self.seen.lock().unwrap().push((true, *kind, *requester))
                    }
                    Event::SensorRelease { kind, requester } => {
                        self.seen.lock().unwrap().push((false, *kind, *requester))
                    }
                    _ => {}
                }
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        r.register(Box::new(Sink { seen: seen.clone() }));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);

        r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);
        r.dispatch_event(Event::CommsActive { pid: 10, active: false }, &mut store);

        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                (true, SensorKind::Proximity, 10),
                (false, SensorKind::Proximity, 10),
            ]
        );
    }

    fn display_power(eff: &[Effect]) -> Vec<bool> {
        eff.iter()
            .filter_map(|e| match e {
                Effect::SetDisplayPower { on } => Some(*on),
                _ => None,
            })
            .collect()
    }

    /// Task 78 — proximity blanks the panel only during a call; uncover restores it.
    #[test]
    fn proximity_blanks_only_during_call() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);

        // No call yet: a near reading must NOT blank.
        let eff = r.dispatch_event(Event::ProximityChanged { near: true }, &mut store);
        assert_eq!(display_power(&eff), Vec::<bool>::new(), "no blank without a call");

        // Call active → near blanks (off), far restores (on).
        r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);
        let eff = r.dispatch_event(Event::ProximityChanged { near: true }, &mut store);
        assert_eq!(display_power(&eff), vec![false], "near in-call → OFF");
        // Repeat near = no-op (transition-only).
        let eff = r.dispatch_event(Event::ProximityChanged { near: true }, &mut store);
        assert_eq!(display_power(&eff), Vec::<bool>::new(), "repeat near = no toggle");
        // Far → restore.
        let eff = r.dispatch_event(Event::ProximityChanged { near: false }, &mut store);
        assert_eq!(display_power(&eff), vec![true], "far → ON");
    }

    /// Task 78 fail-safe — call ending while blanked restores the panel.
    #[test]
    fn call_end_while_blanked_restores_panel() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);
        r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);
        r.dispatch_event(Event::ProximityChanged { near: true }, &mut store); // blanked
        // Hang up while still covered → panel must come back on.
        let eff = r.dispatch_event(Event::CommsActive { pid: 10, active: false }, &mut store);
        assert_eq!(display_power(&eff), vec![true], "call end while blanked → ON");
    }

    /// Task 78 fail-safe — call host dying while blanked restores the panel.
    #[test]
    fn surface_removed_while_blanked_restores_panel() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);
        r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);
        r.dispatch_event(Event::ProximityChanged { near: true }, &mut store); // blanked
        let eff = r.dispatch_event(Event::SurfaceRemoved { pid: 10 }, &mut store);
        assert_eq!(display_power(&eff), vec![true], "call host death while blanked → ON");
    }

    /// Task 81 — POWER key toggles the panel: on→off→on, each press a SetDisplayPower.
    #[test]
    fn power_key_toggles_panel() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        let (_reply, eff) = r.dispatch_command("power-key", "100", &mut store).unwrap();
        assert_eq!(display_power(&eff), vec![false], "first press → OFF");
        let (_reply, eff) = r.dispatch_command("power-key", "100", &mut store).unwrap();
        assert_eq!(display_power(&eff), vec![true], "second press → ON");
    }

    /// Task 81 — `panel on|off` sets explicit power (boot force-on / scripting).
    #[test]
    fn panel_explicit_off_then_on() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        let (_r, eff) = r.dispatch_command("panel", "off", &mut store).unwrap();
        assert_eq!(display_power(&eff), vec![false]);
        let (_r, eff) = r.dispatch_command("panel", "on", &mut store).unwrap();
        assert_eq!(display_power(&eff), vec![true]);
    }

    /// Task 81 — a proximity uncover restores to the user-facing `panel_on`, not
    /// unconditionally on: power off during a call then a near/far cycle keeps it off.
    #[test]
    fn proximity_uncover_restores_to_panel_state() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);
        r.dispatch_command("panel", "off", &mut store).unwrap(); // panel_on = false
        r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);
        let eff = r.dispatch_event(Event::ProximityChanged { near: true }, &mut store);
        assert_eq!(display_power(&eff), vec![false], "near → OFF");
        let eff = r.dispatch_event(Event::ProximityChanged { near: false }, &mut store);
        assert_eq!(display_power(&eff), vec![false], "far → restore to panel_on=off");
    }

    fn suppress_lines(eff: &[Effect]) -> Vec<String> {
        eff.iter()
            .filter_map(|e| match e {
                Effect::HostLine { line, .. } if line.starts_with("input-suppress") => {
                    Some(line.trim().to_string())
                }
                _ => None,
            })
            .collect()
    }

    /// Task 79 — a proximity blank fans `input-suppress 1` to every tracked host
    /// alongside the panel-off, and unblank fans `input-suppress 0`.
    #[test]
    fn proximity_blank_fans_input_suppress() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        add_app(&mut store, "sig", 10);
        add_app(&mut store, "bar", 20);
        r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);

        // Near in-call → panel OFF + suppress fanned to BOTH hosts.
        let eff = r.dispatch_event(Event::ProximityChanged { near: true }, &mut store);
        assert_eq!(display_power(&eff), vec![false]);
        assert_eq!(
            suppress_lines(&eff),
            vec!["input-suppress 1".to_string(), "input-suppress 1".to_string()]
        );

        // Far → panel ON + suppress cleared on both.
        let eff = r.dispatch_event(Event::ProximityChanged { near: false }, &mut store);
        assert_eq!(display_power(&eff), vec![true]);
        assert_eq!(
            suppress_lines(&eff),
            vec!["input-suppress 0".to_string(), "input-suppress 0".to_string()]
        );
    }

    // ── Auto-brightness (task 86) ────────────────────────────────────────────

    /// A typical ALS descriptor (max_range in lux).
    fn seed_light(store: &mut Store) {
        store.set_sensor_descriptor(SensorKind::Light, 40_000.0, 1.0);
    }
    fn light(lux: f32) -> Event {
        Event::SensorReading { kind: SensorKind::Light, x: lux, y: 0.0, z: 0.0, ts_ns: 0 }
    }
    fn backlight(eff: &[Effect]) -> Vec<f32> {
        eff.iter()
            .filter_map(|e| match e {
                Effect::SetBacklight { level, .. } => Some(*level),
                _ => None,
            })
            .collect()
    }

    /// The curve is monotonic in lux and clamped to `[MIN_FRACTION, 1.0]`, with the
    /// ceiling at the sensor's own `max_range` (no hardcoded lux).
    #[test]
    fn lux_curve_monotonic_and_clamped() {
        let max = 40_000.0;
        assert!((lux_to_fraction(0.0, max) - MIN_FRACTION).abs() < 1e-6, "0 lux → floor");
        assert!((lux_to_fraction(max, max) - 1.0).abs() < 1e-6, "max lux → full");
        assert!(lux_to_fraction(-5.0, max) >= MIN_FRACTION, "negative clamps to floor");
        let (a, b, c) = (
            lux_to_fraction(50.0, max),
            lux_to_fraction(500.0, max),
            lux_to_fraction(5000.0, max),
        );
        assert!(a < b && b < c, "monotonic increasing: {a} < {b} < {c}");
        assert!((MIN_FRACTION..=1.0).contains(&a));
        assert_eq!(lux_to_fraction(100.0, 0.0), MIN_FRACTION, "degenerate descriptor → floor");
    }

    /// The curve ceiling is a policy constant (not the sensor descriptor), so a
    /// reading maps + emits even with no descriptor seeded.
    #[test]
    fn auto_brightness_emits_without_descriptor() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        let eff = r.dispatch_event(light(500.0), &mut store);
        assert_eq!(backlight(&eff).len(), 1, "maps via FULL_SCALE_LUX, no descriptor needed");
    }

    /// `brightness-scale` widens the dark→bright spread: a fixed mid lux yields a
    /// higher fraction at a lower full-scale ceiling, and the change applies live.
    #[test]
    fn brightness_scale_steepens_curve() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        // Establish an ambient reading at the default ceiling.
        let hi_ceiling = backlight(&r.dispatch_event(light(100.0), &mut store));
        assert_eq!(hi_ceiling.len(), 1);
        // Lower the ceiling → the SAME 100 lux now maps brighter (recomputed live).
        let (_reply, eff) = r.dispatch_command("brightness-scale", "150", &mut store).unwrap();
        let lo_ceiling = backlight(&eff);
        assert_eq!(lo_ceiling.len(), 1, "scale change re-applies from last lux");
        assert!(lo_ceiling[0] > hi_ceiling[0], "lower ceiling → brighter at same lux ({} > {})", lo_ceiling[0], hi_ceiling[0]);
        // Bad input rejected.
        assert!(matches!(r.dispatch_command("brightness-scale", "0", &mut store).unwrap().0, Reply::Err(_)));
    }

    /// First reading snaps + emits; a brighter ambient pushes the level up (EMA).
    #[test]
    fn auto_brightness_emits_and_tracks_up() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        seed_light(&mut store);
        let first = backlight(&r.dispatch_event(light(50.0), &mut store));
        assert_eq!(first.len(), 1, "first reading emits");
        let second = backlight(&r.dispatch_event(light(20_000.0), &mut store));
        assert_eq!(second.len(), 1);
        assert!(second[0] > first[0], "brighter ambient → higher backlight ({} > {})", second[0], first[0]);
    }

    /// A sub-`MIN_STEP` change after smoothing must not reach the hardware (no flicker).
    #[test]
    fn auto_brightness_hysteresis_suppresses_jitter() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        seed_light(&mut store);
        r.dispatch_event(light(500.0), &mut store); // first emit
        let eff = r.dispatch_event(light(505.0), &mut store); // tiny change
        assert!(backlight(&eff).is_empty(), "sub-JND jitter suppressed");
    }

    /// No auto-brightness while the panel is off or proximity-blanked.
    #[test]
    fn auto_brightness_gated_off_and_blanked() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        seed_light(&mut store);
        add_app(&mut store, "sig", 10);

        r.dispatch_command("panel", "off", &mut store).unwrap();
        let eff = r.dispatch_event(light(5_000.0), &mut store);
        assert!(backlight(&eff).is_empty(), "panel off → no auto-brightness");

        r.dispatch_command("panel", "on", &mut store).unwrap();
        r.dispatch_event(Event::CommsActive { pid: 10, active: true }, &mut store);
        r.dispatch_event(Event::ProximityChanged { near: true }, &mut store); // blanked
        let eff = r.dispatch_event(light(8_000.0), &mut store);
        assert!(backlight(&eff).is_empty(), "blanked → no auto-brightness");
    }

    /// `brightness <f>` pins the level and suppresses auto; `brightness auto` returns
    /// to ambient tracking and re-applies the current ambient level.
    #[test]
    fn manual_override_then_auto() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        seed_light(&mut store);
        r.dispatch_event(light(500.0), &mut store); // ambient established

        let (_reply, eff) = r.dispatch_command("brightness", "0.3", &mut store).unwrap();
        assert_eq!(backlight(&eff), vec![0.3], "manual pins level");
        let eff = r.dispatch_event(light(20_000.0), &mut store);
        assert!(backlight(&eff).is_empty(), "manual override suppresses auto");
        let (_reply, eff) = r.dispatch_command("brightness", "auto", &mut store).unwrap();
        assert_eq!(backlight(&eff).len(), 1, "auto re-applies ambient");

        let bad = r.dispatch_command("brightness", "9", &mut store).unwrap().0;
        assert!(matches!(bad, Reply::Err(_)), "out-of-range rejected");
    }

    /// Waking the panel re-applies the ambient level (not the boot default).
    #[test]
    fn wake_reapplies_ambient_level() {
        let mut r = Registry::new();
        r.register(Box::new(PowerModule::new()));
        let mut store = Store::new();
        seed_light(&mut store);
        r.dispatch_event(light(5_000.0), &mut store); // ambient established
        r.dispatch_command("panel", "off", &mut store).unwrap();
        let (_reply, eff) = r.dispatch_command("panel", "on", &mut store).unwrap();
        let bl = backlight(&eff);
        assert_eq!(bl.len(), 1, "wake re-applies ambient backlight");
        assert!(bl[0] > MIN_FRACTION, "ambient level, not floor");
    }

    // ── Screen-off timeout (PowerManager role, task 86 follow-on) ─────────────

    #[test]
    fn idle_should_sleep_boundary() {
        assert!(!idle_should_sleep(Duration::from_secs(59), Duration::from_secs(60)));
        assert!(idle_should_sleep(Duration::from_secs(60), Duration::from_secs(60)));
        assert!(idle_should_sleep(Duration::from_secs(61), Duration::from_secs(60)));
    }

    /// Input idle past the timeout → an IdleTick sleeps the panel (SetDisplayPower off).
    #[test]
    fn idle_past_timeout_sleeps_panel() {
        let mut r = Registry::new();
        let mut m = PowerModule::new();
        m.last_activity = Some(Instant::now() - Duration::from_millis(DEFAULT_SCREEN_OFF_TIMEOUT_MS + 1000));
        r.register(Box::new(m));
        let mut store = Store::new();
        let eff = r.dispatch_event(Event::IdleTick, &mut store);
        assert_eq!(display_power(&eff), vec![false], "idle past timeout → panel OFF");
    }

    /// A `user-activity` poke resets the idle clock, so the next tick doesn't sleep.
    #[test]
    fn user_activity_resets_idle() {
        let mut r = Registry::new();
        let mut m = PowerModule::new();
        m.last_activity = Some(Instant::now() - Duration::from_millis(DEFAULT_SCREEN_OFF_TIMEOUT_MS + 1000));
        r.register(Box::new(m));
        let mut store = Store::new();
        r.dispatch_command("user-activity", "", &mut store).unwrap();
        let eff = r.dispatch_event(Event::IdleTick, &mut store);
        assert!(display_power(&eff).is_empty(), "recent activity → no sleep");
    }

    /// `screen-timeout off` disables auto-sleep entirely.
    #[test]
    fn screen_timeout_off_never_sleeps() {
        let mut r = Registry::new();
        let mut m = PowerModule::new();
        m.last_activity = Some(Instant::now() - Duration::from_secs(3600));
        r.register(Box::new(m));
        let mut store = Store::new();
        r.dispatch_command("screen-timeout", "off", &mut store).unwrap();
        let eff = r.dispatch_event(Event::IdleTick, &mut store);
        assert!(display_power(&eff).is_empty(), "timeout off → never sleeps");
    }

    /// No auto-sleep when the panel is already off (idempotent) or proximity-blanked.
    #[test]
    fn no_idle_sleep_while_off() {
        let mut r = Registry::new();
        let mut m = PowerModule::new();
        m.panel_on = false;
        m.last_activity = Some(Instant::now() - Duration::from_secs(3600));
        r.register(Box::new(m));
        let mut store = Store::new();
        let eff = r.dispatch_event(Event::IdleTick, &mut store);
        assert!(display_power(&eff).is_empty(), "already off → no-op");
    }

    /// Waking via POWER resets the countdown, so the next tick doesn't immediately re-sleep.
    #[test]
    fn wake_resets_idle_countdown() {
        let mut r = Registry::new();
        let mut m = PowerModule::new();
        m.panel_on = false; // asleep
        r.register(Box::new(m));
        let mut store = Store::new();
        r.dispatch_command("power-key", "", &mut store).unwrap(); // wake (on)
        let eff = r.dispatch_event(Event::IdleTick, &mut store);
        assert!(display_power(&eff).is_empty(), "just woke → no immediate re-sleep");
    }
}
