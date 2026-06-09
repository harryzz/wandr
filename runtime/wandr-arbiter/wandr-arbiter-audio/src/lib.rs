//! wandr-arbiter-audio — the AudioService role (Arbiter Inc.).
//!
//! **M1: the audio-focus stack.** Audio focus is *the* canonical cross-app
//! arbitration: a guest cannot decide to pause another guest, so a central
//! authority must. This module owns the focus stack and decides, on each
//! request, what happens to the previous owner — permanent **loss** (a `gain`
//! request evicts it), **loss-transient** (a `gain-transient` pauses it), or
//! **duck** (a `gain-transient-may-duck` lowers its volume) — and restores the
//! prior owner with **gain** when the top entry abandons. Each transition is an
//! `on-focus-changed <kind>` push to the affected host (`deliver_to_host`); the
//! guest pauses / ducks / resumes. The host/guest WIT wiring is M2; the
//! comms-session (IN_COMMUNICATION) + routing appliers are M3.
//!
//! Focus is runtime-only (it dies with a reboot), so the stack lives in this
//! module, not the durable core Store.

use wandr_arbiter_core::{ArbiterModule, Ctx, Effect, Event, Reply, SensorKind, PRIMARY_DISPLAY};

/// What a guest requests (Android AUDIOFOCUS_GAIN family).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusKind {
    /// Permanent — evict every prior owner (they release).
    Gain,
    /// Transient — the prior owner pauses, resumes when we abandon.
    GainTransient,
    /// Transient may-duck — the prior owner lowers its volume, keeps playing.
    GainTransientMayDuck,
}

impl FocusKind {
    fn from_wire(s: &str) -> Option<Self> {
        match s {
            "gain"                                      => Some(Self::Gain),
            "gain-transient"           | "transient"    => Some(Self::GainTransient),
            "gain-transient-may-duck"  | "duck"         => Some(Self::GainTransientMayDuck),
            _ => None,
        }
    }
    fn as_wire(&self) -> &'static str {
        match self {
            Self::Gain                 => "gain",
            Self::GainTransient        => "gain-transient",
            Self::GainTransientMayDuck => "gain-transient-may-duck",
        }
    }
}

/// What an affected owner is told (Android AUDIOFOCUS_LOSS family), delivered as
/// `on-focus-changed <change>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusChange { Loss, LossTransient, Duck, Gain }

impl FocusChange {
    fn as_wire(&self) -> &'static str {
        match self {
            Self::Loss          => "loss",
            Self::LossTransient => "loss-transient",
            Self::Duck          => "duck",
            Self::Gain          => "gain",
        }
    }
}

#[derive(Clone, Debug)]
struct FocusEntry {
    pid:    i32,
    app_id: String,
    kind:   FocusKind,
}

/// System ringer policy (Android `AudioManager.RINGER_MODE_*`) — what an incoming
/// ring does. The arbiter owns this, like AudioService does in system_server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RingerMode {
    /// Ringtone + vibrate.
    #[default]
    Normal,
    /// Vibrate only.
    Vibrate,
    /// Neither — visual (badge) only.
    Silent,
}

impl RingerMode {
    fn from_wire(s: &str) -> Option<Self> {
        match s {
            "normal"  => Some(Self::Normal),
            "vibrate" => Some(Self::Vibrate),
            "silent"  => Some(Self::Silent),
            _ => None,
        }
    }
    fn as_wire(&self) -> &'static str {
        match self {
            Self::Normal  => "normal",
            Self::Vibrate => "vibrate",
            Self::Silent  => "silent",
        }
    }
}

/// App-assigned notification id the audio module uses for the ongoing-call
/// badge (its own id space, keyed per owner app like any notifier).
const CALL_NOTIF_ID: u64 = 0xCA11;
/// …and the incoming-call (ringing) badge — a distinct id so it can coexist with
/// and be cleared independently of the ongoing-call badge.
const RING_NOTIF_ID: u64 = 0xCA12;

/// The audio-focus arbiter. `stack.last()` is the current owner.
#[derive(Default)]
pub struct AudioModule {
    stack: Vec<FocusEntry>,
    /// M3 — the pid currently in a comms session (a VoIP call), if any. One call
    /// at a time; it holds permanent focus + drives the global audio mode.
    comms: Option<i32>,
    /// The call route the arbiter last decided (`true` = loudspeaker, `false` =
    /// earpiece). Volume keys target this device while a call is up.
    comms_speaker: bool,
    /// Global output-mute state (the arbiter is the single source of truth, so
    /// `toggle` works). Applied by the owner host via the policy `muted` flag.
    muted: bool,
    /// Per-app (per-pid) output-mute set — the apps whose PCM the host should
    /// silence. Orthogonal to `muted` (global): audible iff neither gate is set.
    app_muted: std::collections::HashSet<i32>,
    /// Global mic-mute / input-disable (all apps). The host gates its capture
    /// read path to silence. Single source of truth so `toggle` works.
    mic_muted: bool,
    /// Per-app (per-pid) mic-mute set. Effective mic-mute for an app =
    /// `mic_muted (global) || mic_muted_apps.contains(pid)` — mirrors output.
    mic_muted_apps: std::collections::HashSet<i32>,
    /// The pid with an active incoming-call ring, if any (the Ringer; one at a time).
    ringing: Option<i32>,
    /// System ringer policy — what a ring does (ringtone/vibrate/silent).
    ringer_mode: RingerMode,
}

impl AudioModule {
    pub fn new() -> Self {
        AudioModule::default()
    }

    /// Resolve an owner token to `(pid, app_id)`. A bare integer is a pid (the
    /// host self-reports `getpid()`); resolve its app-id best-effort ("?" if
    /// untracked — focus still works on the raw pid). Otherwise the token is an
    /// app-id (the CLI), resolved to its pid; unknown app-id ⇒ None. Mirrors the
    /// notify/alarm modules.
    fn resolve(token: &str, ctx: &Ctx) -> Option<(i32, String)> {
        match token.parse::<i32>() {
            Ok(pid) => {
                let app_id = ctx.store.app_by_pid(pid)
                    .map(|a| a.app_id.clone())
                    .unwrap_or_else(|| "?".to_string());
                Some((pid, app_id))
            }
            Err(_) => ctx.store.app(token).map(|a| (a.pid, a.app_id.clone())),
        }
    }

    fn push_change(ctx: &mut Ctx, pid: i32, change: FocusChange) {
        ctx.deliver_to_host(pid, format!("on-focus-changed {}\n", change.as_wire()));
    }

    /// `audio-focus-request <pid|app-id> <kind>` — grant focus, demoting the
    /// previous owner per `kind`.
    fn cmd_request(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        if t.len() != 2 {
            return Reply::err("audio-focus-request-args: expected <pid|app-id> <kind>");
        }
        let Some(kind) = FocusKind::from_wire(t[1]) else {
            return Reply::err(format!("audio-focus-bad-kind {:?}", t[1]));
        };
        let Some((pid, app_id)) = Self::resolve(t[0], ctx) else {
            return Reply::err(format!("audio-focus-unknown-owner {}", t[0]));
        };
        self.grant(pid, app_id.clone(), kind, ctx);
        Reply::ok(format!("granted pid={pid} app={app_id} kind={}", kind.as_wire()))
    }

    /// Push `(pid, app_id)` onto the focus stack as the new owner, demoting the
    /// previous owner per `kind` (loss / loss-transient / duck). Shared by the
    /// focus-request verb and the comms-session start (which grabs `Gain`).
    fn grant(&mut self, pid: i32, app_id: String, kind: FocusKind, ctx: &mut Ctx) {
        // Re-request from the same owner: drop its stale entry first so it
        // doesn't get a spurious loss push or a duplicate stack slot.
        self.stack.retain(|e| e.pid != pid);
        let prev = self.stack.last().cloned();

        match kind {
            FocusKind::Gain => {
                // Permanent grab: every prior owner releases.
                for e in &self.stack {
                    Self::push_change(ctx, e.pid, FocusChange::Loss);
                }
                self.stack.clear();
            }
            FocusKind::GainTransient => {
                if let Some(p) = &prev {
                    Self::push_change(ctx, p.pid, FocusChange::LossTransient);
                }
            }
            FocusKind::GainTransientMayDuck => {
                if let Some(p) = &prev {
                    Self::push_change(ctx, p.pid, FocusChange::Duck);
                }
            }
        }

        self.stack.push(FocusEntry { pid, app_id: app_id.clone(), kind });
        log::info!("arbiter: audio-focus granted pid={pid} app={app_id} kind={}", kind.as_wire());
    }

    /// `audio-focus-abandon <pid|app-id>` — release focus; if it was the owner,
    /// the prior (transient/duck) owner underneath regains.
    fn cmd_abandon(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let token = args.trim();
        if token.is_empty() {
            return Reply::err("audio-focus-abandon-args: expected <pid|app-id>");
        }
        let Some((pid, _app)) = Self::resolve(token, ctx) else {
            return Reply::err(format!("audio-focus-unknown-owner {token}"));
        };
        self.drop_pid(pid, ctx);
        Reply::ok(format!("abandoned pid={pid}"))
    }

    /// Remove `pid` from the stack; if it was the current owner, the new top
    /// regains focus. Shared by abandon + the death-watch (SurfaceRemoved).
    fn drop_pid(&mut self, pid: i32, ctx: &mut Ctx) -> bool {
        let was_owner = self.stack.last().map(|e| e.pid) == Some(pid);
        let existed = self.stack.iter().any(|e| e.pid == pid);
        if !existed {
            return false;
        }
        self.stack.retain(|e| e.pid != pid);
        if was_owner {
            if let Some(top) = self.stack.last() {
                Self::push_change(ctx, top.pid, FocusChange::Gain);
                log::info!("arbiter: audio-focus restored pid={} app={}", top.pid, top.app_id);
            }
        }
        true
    }

    /// `audio-call-start <pid|app-id>` — begin a comms session (a VoIP call):
    /// grab transient focus (music pauses, resuming after the call), apply the
    /// call-audio mode recipe (the owner host runs `AudioService.onUpdateAudioMode`
    /// = setPhoneState IN_COMMUNICATION + volume re-apply), mark the session active
    /// (`Event::CommsActive` → proximity-screen-off + doze keep-alive), and raise
    /// an ongoing-call notification. One call at a time.
    ///
    /// IN_COMMUNICATION is *required*: on this device the call's earpiece output
    /// only opens in comms mode (NORMAL → `-889`), and the framework re-applies
    /// volume on the mode flip so it isn't ducked (the host's `on_update_audio_mode`
    /// does both — see audio_policy_impl). `CommsActive` (the screen-off signal) is
    /// decoupled from the audio *route*, not from the *mode*: both fire here.
    fn cmd_call_start(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let token = args.trim();
        let Some((pid, app_id)) = Self::resolve(token, ctx) else {
            return Reply::err(format!("audio-call-unknown-owner {token}"));
        };
        // Answering a ringing call: stop the ring (keep focus — we re-grant below).
        self.stop_ring(pid, &app_id, ctx);
        // Transient focus → music pauses now and resumes when the call ends
        // (Android telephony uses GAIN_TRANSIENT for exactly this).
        self.grant(pid, app_id.clone(), FocusKind::GainTransient, ctx);
        self.comms = Some(pid);
        // The owner host applies the audio-mode recipe (onUpdateAudioMode).
        ctx.deliver_to_host(pid, "audio-policy set-mode comm\n");
        // Ongoing-call badge in the status bar (notify module's Store).
        ctx.store.post_notification(&app_id, CALL_NOTIF_ID, "Ongoing call".into(), app_id.clone());
        // Arm proximity-screen-off + keep the call host out of doze (the power
        // module reacts: acquires proximity, never dozes this pid). Task 78 / M3b.
        ctx.emit(Event::CommsActive { pid, active: true });
        ctx.request(Effect::Persist);
        log::info!("arbiter: audio-call-start pid={pid} app={app_id} → IN_COMMUNICATION + comms session");
        Reply::ok(format!("call-start pid={pid} app={app_id}"))
    }

    /// `audio-call-end <pid|app-id>` — end the comms session: release focus (the
    /// prior owner regains), clear the badge, drop the proximity/doze keep-alive.
    fn cmd_call_end(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let token = args.trim();
        let Some((pid, app_id)) = Self::resolve(token, ctx) else {
            return Reply::err(format!("audio-call-unknown-owner {token}"));
        };
        if self.comms != Some(pid) {
            return Reply::err(format!("audio-call-not-in-session pid={pid}"));
        }
        self.comms = None;
        // Restore NORMAL mode on the owner host (onUpdateAudioMode → setPhoneState).
        ctx.deliver_to_host(pid, "audio-policy set-mode normal\n");
        self.drop_pid(pid, ctx); // release focus → prior owner regains
        ctx.store.cancel_notification(&app_id, CALL_NOTIF_ID);
        // Release proximity + the doze keep-alive (the power module reacts). M3b.
        ctx.emit(Event::CommsActive { pid, active: false });
        ctx.request(Effect::Persist);
        log::info!("arbiter: audio-call-end pid={pid} app={app_id} → NORMAL");
        Reply::ok(format!("call-end pid={pid} app={app_id}"))
    }

    // ── Ringer (incoming-call ringtone + vibrate) ───────────────────────────
    // The system_server analog: Telecom's Ringer + AudioService + VibratorService.
    // The arbiter decides (ringer mode); the owner host plays the ringtone (its
    // `audio` interface) and vibrates (its `haptics` interface).

    /// `audio-ring-start <pid|app-id>` — an incoming call is ringing. Per the
    /// ringer mode: ringtone + vibrate (normal), vibrate only (vibrate), or silent.
    /// Pauses music (transient focus) + raises an incoming-call badge. Stopped by
    /// `audio-ring-stop` (decline/miss) or `audio-call-start` (answer).
    fn cmd_ring_start(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let token = args.trim();
        let Some((pid, app_id)) = Self::resolve(token, ctx) else {
            return Reply::err(format!("audio-ring-unknown-owner {token}"));
        };
        self.ringing = Some(pid);
        // PRE-WARM proximity during the ring (task: the platform `enableSensor` pays a
        // ~5 s timeout re-activating a COLD sensor — the SLPI powers it down with no
        // client). Acquiring it now, while the phone rings, means it's already warm by
        // the time the call is answered (`cmd_call_start` → CommsActive re-acquires the
        // SAME pid → deduped, so the hold persists into the call). Released on
        // decline/miss (`cmd_ring_stop`); on answer the ring's hold rides into the call
        // and is dropped at call-end. (`stop_ring`, shared by answer + decline, does
        // NOT touch proximity, so answering keeps it warm.)
        ctx.emit(Event::SensorAcquire { kind: SensorKind::Proximity, requester: pid });
        // The ringtone pauses music; it resumes when the ring stops (Android uses
        // GAIN_TRANSIENT for the ring stream).
        self.grant(pid, app_id.clone(), FocusKind::GainTransient, ctx);
        match self.ringer_mode {
            RingerMode::Normal => {
                ctx.deliver_to_host(pid, "ringtone start\n");
                ctx.deliver_to_host(pid, "haptics ring-start\n");
            }
            RingerMode::Vibrate => ctx.deliver_to_host(pid, "haptics ring-start\n"),
            RingerMode::Silent => {}
        }
        ctx.store.post_notification(&app_id, RING_NOTIF_ID, "Incoming call".into(), app_id.clone());
        ctx.request(Effect::Persist);
        log::info!("arbiter: audio-ring-start pid={pid} app={app_id} mode={}", self.ringer_mode.as_wire());
        Reply::ok(format!("ring-start pid={pid} app={app_id} mode={}", self.ringer_mode.as_wire()))
    }

    /// `audio-ring-stop <pid|app-id>` — the incoming call was declined / missed /
    /// canceled (not answered). Stop the ring + release focus (music resumes).
    fn cmd_ring_stop(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let token = args.trim();
        let Some((pid, app_id)) = Self::resolve(token, ctx) else {
            return Reply::err(format!("audio-ring-unknown-owner {token}"));
        };
        if self.ringing != Some(pid) {
            return Reply::err(format!("audio-ring-not-ringing pid={pid}"));
        }
        self.stop_ring(pid, &app_id, ctx);
        // Declined / missed (not answered): release the pre-warm acquired in
        // cmd_ring_start. (On ANSWER this path isn't taken — cmd_call_start stops the
        // ring directly + CommsActive holds proximity, so the warm sensor rides in.)
        ctx.emit(Event::SensorRelease { kind: SensorKind::Proximity, requester: pid });
        self.drop_pid(pid, ctx); // release transient focus → prior owner resumes
        ctx.request(Effect::Persist);
        log::info!("arbiter: audio-ring-stop pid={pid} app={app_id}");
        Reply::ok(format!("ring-stop pid={pid} app={app_id}"))
    }

    /// `audio-ringer-mode <normal|vibrate|silent>` — set the system ringer policy
    /// (Android `AudioManager.setRingerMode`). Applies to subsequent rings.
    fn cmd_ringer_mode(&mut self, args: &str, _ctx: &mut Ctx) -> Reply {
        match RingerMode::from_wire(args.trim()) {
            Some(m) => {
                self.ringer_mode = m;
                log::info!("arbiter: ringer-mode → {}", m.as_wire());
                Reply::ok(format!("ringer-mode {}", m.as_wire()))
            }
            None => Reply::err("audio-ringer-mode: expected normal|vibrate|silent"),
        }
    }

    /// Stop the active ring's sounds + clear its badge (does NOT touch focus — the
    /// caller decides whether to release it: `audio-ring-stop` releases, an answer
    /// via `audio-call-start` keeps it). No-op if this pid isn't ringing.
    fn stop_ring(&mut self, pid: i32, app_id: &str, ctx: &mut Ctx) {
        if self.ringing != Some(pid) {
            return;
        }
        self.ringing = None;
        // Always send both stops (idempotent on the host even if one wasn't started).
        ctx.deliver_to_host(pid, "ringtone stop\n");
        ctx.deliver_to_host(pid, "haptics ring-stop\n");
        ctx.store.cancel_notification(app_id, RING_NOTIF_ID);
    }

    /// `audio-route <pid|app-id> <speaker|earpiece>` — the speaker toggle: the
    /// owner host applies `setForceUse(COMMUNICATION, …)`.
    fn cmd_route(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        if t.len() != 2 {
            return Reply::err("audio-route-args: expected <pid|app-id> <speaker|earpiece>");
        }
        let route = match t[1] {
            "speaker" | "earpiece" => t[1],
            other => return Reply::err(format!("audio-route-bad-target {other:?}")),
        };
        let Some((pid, _app)) = Self::resolve(t[0], ctx) else {
            return Reply::err(format!("audio-route-unknown-owner {}", t[0]));
        };
        self.comms_speaker = route == "speaker";
        ctx.deliver_to_host(pid, format!("audio-policy set-route {route}\n"));
        log::info!("arbiter: audio-route pid={pid} → {route}");
        Reply::ok(format!("route pid={pid} {route}"))
    }

    /// `play-tone [pid|app-id] [ms] [hz] [vol0-1]` — tell a host (the audio applier)
    /// to play a sine tone. Arbiter decides → host applies (same path as audio-route):
    /// a runtime/CLI way to make a sound + warm the audio output path. Output-only on
    /// the host side (no capture → no in+out MMAP -889).
    ///
    /// The target is OPTIONAL: the arbiter doesn't open audio itself, so the tone
    /// runs *inside a host process*. With no target it defaults to the foreground
    /// app's host (`visible_app`) — the one the user is looking at. A leading
    /// `pid|app-id` overrides that (e.g. to warm a specific app's output). The
    /// remaining positionals are `ms hz vol`. Defaults: 1500 ms, 440 Hz, 0.5.
    fn cmd_play_tone(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let mut t: Vec<&str> = args.split_whitespace().collect();
        // First token is a target only if it names a host the store actually
        // tracks — a known app-id, or a KNOWN pid (not just any integer, since
        // `resolve` maps unknown pids to "?"). Otherwise it's the start of the
        // numeric `ms hz vol` triple and the target defaults to the foreground
        // host. Keeps `play-tone`, `play-tone 2000 660 0.5`, and
        // `play-tone wandr.launcher 2000` all unambiguous.
        let first_is_target = t.first().is_some_and(|tok| match tok.parse::<i32>() {
            Ok(pid) => ctx.store.app_by_pid(pid).is_some(),
            Err(_)  => ctx.store.app(tok).is_some(),
        });
        let pid = if first_is_target {
            let (pid, _app) = Self::resolve(t.remove(0), ctx).expect("known target resolves");
            pid
        } else {
            match ctx.store.display(PRIMARY_DISPLAY).and_then(|d| d.visible_app()) {
                Some(pid) => pid,
                None => return Reply::err("play-tone-no-foreground: no visible app; pass <pid|app-id>"),
            }
        };
        let ms  = t.first().copied().unwrap_or("1500");
        let hz  = t.get(1).copied().unwrap_or("440");
        let vol = t.get(2).copied().unwrap_or("0.5");
        ctx.deliver_to_host(pid, format!("play-tone {ms} {hz} {vol}\n"));
        log::info!("arbiter: play-tone pid={pid} ms={ms} hz={hz} vol={vol}");
        Reply::ok(format!("play-tone pid={pid} ms={ms} hz={hz} vol={vol}"))
    }

    /// `volume <up|down>` — task 76 P8. The arbiter is the single decider: it
    /// owns the call/comms state + route + foreground, so it picks the target
    /// (the comms owner on the call route while a call is up, else the
    /// foreground app on the loudspeaker) and tells exactly **one** host to
    /// apply the step. This dedups the key (which the framework delivers to
    /// several wandr surfaces) and keeps the choice correct regardless of which
    /// surface caught it. Host applies via `audio-policy volume <dir> <dev>`.
    /// Resolve the host + device an audio control should act on: the comms owner
    /// on the call route while a call is up, else the foreground app, else the
    /// forwarding host (`sender`, so a control still works with no Foreground
    /// slot — e.g. keyguard locked). The single place the "who/where" is decided.
    fn audio_target(&self, ctx: &mut Ctx, sender: Option<i32>) -> Option<(i32, &'static str)> {
        if let Some(pid) = self.comms {
            return Some((pid, if self.comms_speaker { "speaker" } else { "earpiece" }));
        }
        ctx.store.display(PRIMARY_DISPLAY)
            .and_then(|d| d.foreground_slot()).map(|s| s.pid)
            .or(sender)
            .map(|pid| (pid, "speaker"))
    }

    fn cmd_volume(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        // "<up|down> [sender-pid]" — the host forwards its own pid so we always
        // have a live applier even when there's no Foreground slot (e.g. while
        // the keyguard is locked). During a call we override to the comms owner.
        let t: Vec<&str> = args.split_whitespace().collect();
        let up = match t.first().copied() {
            Some("up")   => true,
            Some("down") => false,
            other        => return Reply::err(format!("volume-bad-dir {other:?}")),
        };
        let sender = t.get(1).and_then(|p| p.parse::<i32>().ok());
        let Some((owner, dev)) = self.audio_target(ctx, sender) else {
            return Reply::err("volume-no-target");
        };
        let dir = if up { "up" } else { "down" };
        ctx.deliver_to_host(owner, format!("audio-policy volume {dir} {dev}\n"));
        log::info!("arbiter: volume {dir} → pid={owner} {dev}");
        Reply::ok(format!("volume {dir} pid={owner} {dev}"))
    }

    /// `mute <on|off|toggle> [sender-pid]` — task 76. Output mute, arbiter-owned
    /// (so `toggle` has a single source of truth). Same target resolution as
    /// volume; the owner host applies it via the policy volume setter's `muted`.
    fn cmd_mute(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        let new = match t.first().copied() {
            Some("on")     => true,
            Some("off")    => false,
            Some("toggle") => !self.muted,
            other          => return Reply::err(format!("mute-bad-arg {other:?}")),
        };
        let sender = t.get(1).and_then(|p| p.parse::<i32>().ok());
        let Some((owner, dev)) = self.audio_target(ctx, sender) else {
            return Reply::err("mute-no-target");
        };
        self.muted = new;
        let state = if new { "on" } else { "off" };
        ctx.deliver_to_host(owner, format!("audio-policy mute {state} {dev}\n"));
        log::info!("arbiter: mute {state} → pid={owner} {dev}");
        Reply::ok(format!("mute {state} pid={owner} {dev}"))
    }

    /// Effective mic-mute for `pid` = global OR per-app (mirrors output mute).
    fn mic_effective(&self, pid: i32) -> bool {
        self.mic_muted || self.mic_muted_apps.contains(&pid)
    }

    /// `mic-mute <on|off|toggle>` — task 76. GLOBAL input mute / mic-disable
    /// (all apps), the input-side twin of `mute`. Since mic capture is host-
    /// owned per process (no system mic-mute binding), "global" fans the
    /// effective state out to every running app host. (Apps launched *during* a
    /// global mute don't auto-inherit — minor, mic capture is call-only; revisit
    /// if needed.) Arbiter-owned so `toggle` is consistent. Dormant until a
    /// guest opens capture (Signal is RX-only today).
    fn cmd_mic_mute(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let new = match args.trim() {
            "on"     => true,
            "off"    => false,
            "toggle" => !self.mic_muted,
            other    => return Reply::err(format!("mic-mute-bad-arg {other:?}")),
        };
        self.mic_muted = new;
        let pids: Vec<i32> = ctx.store.apps_snapshot().iter().map(|a| a.pid).collect();
        for pid in &pids {
            let eff = self.mic_effective(*pid);
            ctx.deliver_to_host(*pid, format!("audio-policy mic-mute {}\n", if eff { "on" } else { "off" }));
        }
        let state = if new { "on" } else { "off" };
        log::info!("arbiter: mic-mute {state} (global) → {} apps", pids.len());
        Reply::ok(format!("mic-mute {state} global ({} apps)", pids.len()))
    }

    /// `app-mic-mute <pid|app-id> <on|off|toggle>` — task 76. Per-app input
    /// mute (the input-side twin of `app-mute`). Effective = global || this app.
    fn cmd_app_mic_mute(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        if t.len() != 2 {
            return Reply::err("app-mic-mute-args: expected <pid|app-id> <on|off|toggle>");
        }
        let Some((pid, _app)) = Self::resolve(t[0], ctx) else {
            return Reply::err(format!("app-mic-mute-unknown {}", t[0]));
        };
        let new = match t[1] {
            "on"     => true,
            "off"    => false,
            "toggle" => !self.mic_muted_apps.contains(&pid),
            other    => return Reply::err(format!("app-mic-mute-bad-arg {other:?}")),
        };
        if new { self.mic_muted_apps.insert(pid); } else { self.mic_muted_apps.remove(&pid); }
        let eff = self.mic_effective(pid);
        ctx.deliver_to_host(pid, format!("audio-policy mic-mute {}\n", if eff { "on" } else { "off" }));
        log::info!("arbiter: app-mic-mute {} → pid={pid} (eff={eff})", if new { "on" } else { "off" });
        Reply::ok(format!("app-mic-mute {} pid={pid} eff={eff}", if new { "on" } else { "off" }))
    }

    /// `app-mute <pid|app-id> <on|off|toggle>` — task 76. Per-app output mute:
    /// the owner host gates that app's PCM at the source (silence). Orthogonal to
    /// the global `mute`. The arbiter tracks the per-pid state so `toggle` works.
    fn cmd_app_mute(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let t: Vec<&str> = args.split_whitespace().collect();
        if t.len() != 2 {
            return Reply::err("app-mute-args: expected <pid|app-id> <on|off|toggle>");
        }
        let Some((pid, _app)) = Self::resolve(t[0], ctx) else {
            return Reply::err(format!("app-mute-unknown {}", t[0]));
        };
        let new = match t[1] {
            "on"     => true,
            "off"    => false,
            "toggle" => !self.app_muted.contains(&pid),
            other    => return Reply::err(format!("app-mute-bad-arg {other:?}")),
        };
        if new { self.app_muted.insert(pid); } else { self.app_muted.remove(&pid); }
        let state = if new { "on" } else { "off" };
        ctx.deliver_to_host(pid, format!("audio-policy app-mute {state}\n"));
        log::info!("arbiter: app-mute {state} → pid={pid}");
        Reply::ok(format!("app-mute {state} pid={pid}"))
    }

    /// `audio-focus-list` — the stack, owner last. CLI + inspection.
    fn cmd_list(&self) -> Reply {
        let mut out = format!("count={}", self.stack.len());
        for (i, e) in self.stack.iter().enumerate() {
            let marker = if i + 1 == self.stack.len() { " [owner]" } else { "" };
            out.push_str(&format!(
                "\n  pid={} app={} kind={}{marker}",
                e.pid, e.app_id, e.kind.as_wire(),
            ));
        }
        Reply::ok(out)
    }
}

impl ArbiterModule for AudioModule {
    fn verbs(&self) -> &[&'static str] {
        &[
            "audio-focus-request", "audio-focus-abandon", "audio-focus-list",
            "audio-call-start", "audio-call-end", "audio-route",
            "audio-ring-start", "audio-ring-stop", "audio-ringer-mode",
            "volume", "mute", "app-mute", "mic-mute", "app-mic-mute",
            "play-tone",
        ]
    }

    fn on_command(&mut self, verb: &str, args: &str, ctx: &mut Ctx) -> Reply {
        match verb {
            "audio-focus-request" => self.cmd_request(args, ctx),
            "audio-focus-abandon" => self.cmd_abandon(args, ctx),
            "audio-focus-list"    => self.cmd_list(),
            "audio-call-start"    => self.cmd_call_start(args, ctx),
            "audio-call-end"      => self.cmd_call_end(args, ctx),
            "audio-route"         => self.cmd_route(args, ctx),
            "play-tone"           => self.cmd_play_tone(args, ctx),
            "audio-ring-start"    => self.cmd_ring_start(args, ctx),
            "audio-ring-stop"     => self.cmd_ring_stop(args, ctx),
            "audio-ringer-mode"   => self.cmd_ringer_mode(args, ctx),
            "volume"              => self.cmd_volume(args, ctx),
            "mute"                => self.cmd_mute(args, ctx),
            "app-mute"            => self.cmd_app_mute(args, ctx),
            "mic-mute"            => self.cmd_mic_mute(args, ctx),
            "app-mic-mute"        => self.cmd_app_mic_mute(args, ctx),
            other => Reply::err(format!("audio-unknown-verb {other}")),
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut Ctx) {
        // A focus owner's process died — drop it and restore the next owner.
        if let Event::SurfaceRemoved { pid } = ev {
            // If the comms-session owner died, end the session: clear the mode
            // marker + the ongoing-call badge (no set-mode push — the host is
            // gone; the next host to set a mode will correct it).
            if self.comms == Some(*pid) {
                self.comms = None;
                if let Some(app) = ctx.store.app_by_pid(*pid).map(|a| a.app_id.clone()) {
                    ctx.store.cancel_notification(&app, CALL_NOTIF_ID);
                }
                log::info!("arbiter: audio-call owner pid={pid} died — session ended");
            }
            // A ringing owner died → clear the ring + its badge (the host is gone,
            // so no stop push is needed).
            if self.ringing == Some(*pid) {
                self.ringing = None;
                if let Some(app) = ctx.store.app_by_pid(*pid).map(|a| a.app_id.clone()) {
                    ctx.store.cancel_notification(&app, RING_NOTIF_ID);
                }
                log::info!("arbiter: audio-ring owner pid={pid} died — ring cleared");
            }
            // Drop stale per-app mute so a recycled pid doesn't inherit it.
            self.app_muted.remove(pid);
            self.mic_muted_apps.remove(pid);
            if self.drop_pid(*pid, ctx) {
                log::info!("arbiter: audio-focus dropped dead pid={pid}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Instant, SystemTime};
    use wandr_arbiter_core::{AppState, Effect, Registry, Store};

    fn reg() -> (Registry, Store) {
        let mut r = Registry::new();
        r.register(Box::new(AudioModule::new()));
        (r, Store::new())
    }

    fn seed(s: &mut Store, id: &str, pid: i32) {
        s.insert_app(AppState {
            app_id: id.to_string(),
            pid,
            launched_at: SystemTime::now(),
            launched_mono: Instant::now(),
        });
    }

    /// Collect the `on-focus-changed <kind>` pushes to a given pid.
    fn changes_for(eff: &[Effect], pid: i32) -> Vec<String> {
        eff.iter().filter_map(|e| match e {
            Effect::HostLine { pid: p, line } if *p == pid =>
                Some(line.trim().to_string()),
            _ => None,
        }).collect()
    }

    #[test]
    fn transient_pauses_then_restores_owner() {
        let (mut r, mut store) = reg();
        // Music app takes permanent focus.
        let (reply, _) = r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        assert!(matches!(reply, Reply::Ok(_)));
        // A navigation prompt takes transient focus → music gets loss-transient.
        let (_r, eff) = r.dispatch_command("audio-focus-request", "222 gain-transient", &mut store).unwrap();
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed loss-transient"]);
        // Prompt abandons → music regains.
        let (_r, eff) = r.dispatch_command("audio-focus-abandon", "222", &mut store).unwrap();
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed gain"]);
    }

    #[test]
    fn permanent_gain_evicts_everyone() {
        let (mut r, mut store) = reg();
        r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        r.dispatch_command("audio-focus-request", "222 gain-transient", &mut store).unwrap();
        // A call grabs permanent focus → both prior owners get loss.
        let (_r, eff) = r.dispatch_command("audio-focus-request", "333 gain", &mut store).unwrap();
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed loss"]);
        assert_eq!(changes_for(&eff, 222), vec!["on-focus-changed loss"]);
        // Only the call remains; abandoning it restores nothing.
        let (_r, eff) = r.dispatch_command("audio-focus-abandon", "333", &mut store).unwrap();
        assert!(eff.is_empty());
    }

    #[test]
    fn duck_does_not_clear_stack() {
        let (mut r, mut store) = reg();
        r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        let (_r, eff) = r.dispatch_command("audio-focus-request", "222 duck", &mut store).unwrap();
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed duck"]);
        // Owner is 222 with 111 still under it.
        let (reply, _) = r.dispatch_command("audio-focus-list", "", &mut store).unwrap();
        let Reply::Ok(body) = reply else { panic!() };
        assert!(body.contains("count=2"));
    }

    #[test]
    fn app_id_resolves_to_pid() {
        let (mut r, mut store) = reg();
        seed(&mut store, "test.caller", 4321);
        let (reply, _) = r.dispatch_command("audio-focus-request", "test.caller gain", &mut store).unwrap();
        let Reply::Ok(body) = reply else { panic!() };
        assert!(body.contains("pid=4321"));
        assert!(body.contains("app=test.caller"));
    }

    #[test]
    fn dead_owner_is_dropped_and_next_restored() {
        let (mut r, mut store) = reg();
        r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        r.dispatch_command("audio-focus-request", "222 gain-transient", &mut store).unwrap();
        // Owner 222 dies → 111 regains.
        let eff = r.dispatch_event(Event::SurfaceRemoved { pid: 222 }, &mut store);
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed gain"]);
        // A non-owner death just prunes (no restore push).
        r.dispatch_command("audio-focus-request", "333 gain-transient", &mut store).unwrap();
        let eff = r.dispatch_event(Event::SurfaceRemoved { pid: 111 }, &mut store);
        assert!(eff.is_empty());
    }

    #[test]
    fn call_start_pauses_music_and_badges() {
        let (mut r, mut store) = reg();
        seed(&mut store, "test.caller", 500);
        // Music playing (permanent focus).
        r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        let (reply, eff) = r.dispatch_command("audio-call-start", "test.caller", &mut store).unwrap();
        assert!(matches!(reply, Reply::Ok(_)));
        // Music pauses (transient), not permanently evicted.
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed loss-transient"]);
        // The owner host is told to enter the comms audio mode (onUpdateAudioMode).
        assert!(changes_for(&eff, 500).iter().any(|l| l == "audio-policy set-mode comm"));
        // Ongoing-call badge raised.
        assert!(store.notifications().iter().any(|n| n.app_id == "test.caller"));
    }

    #[test]
    fn call_end_resumes_music_clears_badge() {
        let (mut r, mut store) = reg();
        seed(&mut store, "test.caller", 500);
        r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        r.dispatch_command("audio-call-start", "test.caller", &mut store).unwrap();
        let (_r, eff) = r.dispatch_command("audio-call-end", "test.caller", &mut store).unwrap();
        // Mode restored to NORMAL on the owner; music regains focus.
        assert!(changes_for(&eff, 500).iter().any(|l| l == "audio-policy set-mode normal"));
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed gain"]);
        // Badge cleared.
        assert!(store.notifications().iter().all(|n| n.app_id != "test.caller"));
    }

    #[test]
    fn route_targets_the_owner_host() {
        let (mut r, mut store) = reg();
        let (reply, eff) = r.dispatch_command("audio-route", "500 speaker", &mut store).unwrap();
        assert!(matches!(reply, Reply::Ok(_)));
        assert!(changes_for(&eff, 500).iter().any(|l| l == "audio-policy set-route speaker"));
    }

    #[test]
    fn comms_owner_death_ends_session() {
        let (mut r, mut store) = reg();
        seed(&mut store, "test.caller", 500);
        r.dispatch_command("audio-call-start", "test.caller", &mut store).unwrap();
        assert!(store.notifications().iter().any(|n| n.app_id == "test.caller"));
        // Owner dies → session ends, badge cleared.
        r.dispatch_event(Event::SurfaceRemoved { pid: 500 }, &mut store);
        assert!(store.notifications().iter().all(|n| n.app_id != "test.caller"));
    }

    #[test]
    fn rerequest_same_owner_no_self_loss() {
        let (mut r, mut store) = reg();
        r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        // Same pid re-requests (e.g. upgrade transient→gain): no loss to itself.
        let (_r, eff) = r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        assert!(changes_for(&eff, 111).is_empty());
        let (reply, _) = r.dispatch_command("audio-focus-list", "", &mut store).unwrap();
        let Reply::Ok(body) = reply else { panic!() };
        assert!(body.contains("count=1"));
    }

    // ── Ringer ───────────────────────────────────────────────────────────────

    #[test]
    fn ring_start_normal_rings_vibrates_and_pauses_music() {
        let (mut r, mut store) = reg();
        seed(&mut store, "test.caller", 500);
        r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap(); // music
        let (reply, eff) = r.dispatch_command("audio-ring-start", "test.caller", &mut store).unwrap();
        assert!(matches!(reply, Reply::Ok(_)));
        let lines = changes_for(&eff, 500);
        assert!(lines.iter().any(|l| l == "ringtone start"));
        assert!(lines.iter().any(|l| l == "haptics ring-start"));
        // Music pauses (transient), incoming-call badge raised.
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed loss-transient"]);
        assert!(store.notifications().iter().any(|n| n.app_id == "test.caller"));
    }

    #[test]
    fn ring_stop_silences_and_resumes_music() {
        let (mut r, mut store) = reg();
        seed(&mut store, "test.caller", 500);
        r.dispatch_command("audio-focus-request", "111 gain", &mut store).unwrap();
        r.dispatch_command("audio-ring-start", "test.caller", &mut store).unwrap();
        let (_r, eff) = r.dispatch_command("audio-ring-stop", "test.caller", &mut store).unwrap();
        let lines = changes_for(&eff, 500);
        assert!(lines.iter().any(|l| l == "ringtone stop"));
        assert!(lines.iter().any(|l| l == "haptics ring-stop"));
        // Music regains, badge cleared.
        assert_eq!(changes_for(&eff, 111), vec!["on-focus-changed gain"]);
        assert!(store.notifications().iter().all(|n| n.app_id != "test.caller"));
    }

    #[test]
    fn answer_stops_ring_then_starts_session() {
        let (mut r, mut store) = reg();
        seed(&mut store, "test.caller", 500);
        r.dispatch_command("audio-ring-start", "test.caller", &mut store).unwrap();
        // Answering = call-start: it stops the ring + enters the comms audio mode.
        let (_r, eff) = r.dispatch_command("audio-call-start", "test.caller", &mut store).unwrap();
        let lines = changes_for(&eff, 500);
        assert!(lines.iter().any(|l| l == "ringtone stop"));
        assert!(lines.iter().any(|l| l == "audio-policy set-mode comm"));
        // Ongoing-call badge present; not still "ringing".
        assert!(store.notifications().iter().any(|n| n.app_id == "test.caller"));
    }

    #[test]
    fn ringer_mode_vibrate_buzzes_without_ringtone() {
        let (mut r, mut store) = reg();
        seed(&mut store, "test.caller", 500);
        r.dispatch_command("audio-ringer-mode", "vibrate", &mut store).unwrap();
        let (_r, eff) = r.dispatch_command("audio-ring-start", "test.caller", &mut store).unwrap();
        let lines = changes_for(&eff, 500);
        assert!(lines.iter().any(|l| l == "haptics ring-start"));
        assert!(!lines.iter().any(|l| l == "ringtone start"));
    }

    #[test]
    fn ringer_mode_silent_is_visual_only() {
        let (mut r, mut store) = reg();
        seed(&mut store, "test.caller", 500);
        r.dispatch_command("audio-ringer-mode", "silent", &mut store).unwrap();
        let (_r, eff) = r.dispatch_command("audio-ring-start", "test.caller", &mut store).unwrap();
        let lines = changes_for(&eff, 500);
        assert!(!lines.iter().any(|l| l == "ringtone start"));
        assert!(!lines.iter().any(|l| l == "haptics ring-start"));
        // Still badges the incoming call.
        assert!(store.notifications().iter().any(|n| n.app_id == "test.caller"));
    }

    #[test]
    fn ringing_owner_death_clears_ring() {
        let (mut r, mut store) = reg();
        seed(&mut store, "test.caller", 500);
        r.dispatch_command("audio-ring-start", "test.caller", &mut store).unwrap();
        assert!(store.notifications().iter().any(|n| n.app_id == "test.caller"));
        r.dispatch_event(Event::SurfaceRemoved { pid: 500 }, &mut store);
        assert!(store.notifications().iter().all(|n| n.app_id != "test.caller"));
    }
}
