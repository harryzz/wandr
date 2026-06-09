//! call.rs — the engine's Signal 1:1 voice-call adapter (Phase 2b-ii).
//!
//! Bridges libsignal's `CallMessage` (the signaling envelope that rides the
//! existing E2E channel) and the host audio/UDP to the `wandr_call` [`SignalCall`]
//! driver, running one active audio call:
//!
//!   CallMessage ⇄ [`CallSignal`]  +  mic/AAudio (host `audio`)  +  `UdpSocket`
//!
//! The engine's `run()` loop owns a [`CallEngine`]: it feeds inbound
//! `CallMessage`s in, `tick`s it every ~10 ms (UDP + audio pump), and sends the
//! `CallSignal`s it emits back out as `CallMessage`s.
//!
//! **Identity binding (Phase 3):** ringrtc's DH-SRTP HKDF binds the SRTP keys to
//! the two participants' serialized **identity public keys** (33-byte `0x05‖X25519`,
//! caller = offerer first). The engine resolves these from the protocol store —
//! ours via `store.identity()`, the peer's via `store.get_identity(addr)` — and
//! passes the bytes in, so the derived keys match a real Signal client's.

use std::net::{SocketAddr, UdpSocket};
use std::time::Instant;

use libsignal_service::proto::{call_message, CallMessage};
use libsignal_service::push_service::TurnServerInfo;
use wandr_call::local_lan_ip;
use wandr_call::signal::{CallSignal, CallState, HangupKind, SignalCall};
use wandr_call::turn::TurnConfig;

use crate::my::skiko_gfx::audio::{self, ChannelLayout, Format, StreamClass, TrackConfig};
use crate::wandr::audio_focus::controls;

const SAMPLE_RATE: u32 = wandr_call::SAMPLE_RATE;
/// 20 ms @ 48 kHz mono — the Opus frame + audio tick granularity.
const FRAME: usize = SAMPLE_RATE as usize / 1000 * 20;

// Proto enum ints (SignalService.proto `CallMessage`), stable on the wire — used
// directly to avoid depending on prost's generated enum variant names.
const OFFER_AUDIO_CALL: i32 = 0;
const HANGUP_NORMAL: i32 = 0;
const HANGUP_ACCEPTED: i32 = 1;
const HANGUP_DECLINED: i32 = 2;
const HANGUP_BUSY: i32 = 3;

/// Build a wandr-call [`TurnConfig`] from a Signal calling relay. Prefers the
/// IP-pinned URLs (`urlsWithIps`) and requires `ip:port` form — the wasip2 guest has
/// no reliable DNS, and wandr-call demuxes inbound TURN traffic by the server address.
/// Picks a UDP `turn:` server (+ a `stun:` server if present). `None` if none usable.
pub fn turn_config_from(relay: &TurnServerInfo) -> Option<TurnConfig> {
    // Pick a UDP server of `scheme`, preferring IPv4 (our media socket is IPv4).
    let pick = |want: &str| -> Option<std::net::SocketAddr> {
        let (mut v4, mut v6) = (None, None);
        for u in relay.urls_with_ips.iter().chain(relay.urls.iter()) {
            let Some((scheme, addr)) = parse_ice_uri(u) else { continue };
            if scheme != want {
                continue;
            }
            if addr.is_ipv4() {
                v4.get_or_insert(addr);
            } else {
                v6.get_or_insert(addr);
            }
        }
        v4.or(v6)
    };
    let turn = pick("turn")?;
    Some(TurnConfig {
        stun_serv_addr: pick("stun").map(|a| a.to_string()).unwrap_or_default(),
        turn_serv_addr: turn.to_string(),
        username: relay.username.clone(),
        password: relay.password.clone(),
        realm: relay.hostname.clone().unwrap_or_default(),
    })
}

/// Parse a **UDP** ICE URI in IP form → (scheme, `SocketAddr`). Only plain
/// `turn:`/`stun:` (not `turns:`/`stuns:` TLS, not `transport=tcp`) on an IP host;
/// a missing port defaults to 3478 (the STUN/TURN default). `None` otherwise.
fn parse_ice_uri(uri: &str) -> Option<(&'static str, std::net::SocketAddr)> {
    let (scheme, rest) = uri.split_once(':')?;
    let base = match scheme.to_ascii_lowercase().as_str() {
        "turn" => "turn",
        "stun" => "stun",
        _ => return None, // turns:/stuns: are TLS/TCP — skip
    };
    let (hostport, query) = rest.split_once('?').unwrap_or((rest, ""));
    if query.contains("transport=tcp") {
        return None;
    }
    // Accept `ip:port`, or default the port to 3478 for a bare `ip` (incl. IPv6).
    let addr = hostport
        .parse::<std::net::SocketAddr>()
        .or_else(|_| format!("{hostport}:3478").parse::<std::net::SocketAddr>())
        .ok()?;
    Some((base, addr))
}

/// Map one outgoing [`CallSignal`] to a libsignal `CallMessage`.
pub fn signal_to_call_message(sig: &CallSignal) -> CallMessage {
    let mut cm = CallMessage::default();
    match sig {
        CallSignal::Offer { call_id, opaque } => {
            cm.offer = Some(call_message::Offer {
                id: Some(*call_id),
                r#type: Some(OFFER_AUDIO_CALL),
                opaque: Some(opaque.clone()),
            });
        }
        CallSignal::Answer { call_id, opaque } => {
            cm.answer = Some(call_message::Answer { id: Some(*call_id), opaque: Some(opaque.clone()) });
        }
        CallSignal::Ice { call_id, opaque } => {
            cm.ice_update =
                vec![call_message::IceUpdate { id: Some(*call_id), opaque: Some(opaque.clone()) }];
        }
        CallSignal::Hangup { call_id, kind } => {
            cm.hangup = Some(call_message::Hangup {
                id: Some(*call_id),
                r#type: Some(hangup_type(*kind)),
                device_id: None,
            });
        }
        CallSignal::Busy { call_id } => {
            cm.busy = Some(call_message::Busy { id: Some(*call_id) });
        }
    }
    cm
}

/// Map an inbound libsignal `CallMessage` to the [`CallSignal`]s it carries.
pub fn call_message_to_signals(cm: &CallMessage) -> Vec<CallSignal> {
    let mut out = Vec::new();
    if let Some(o) = &cm.offer {
        if let (Some(id), Some(op)) = (o.id, o.opaque.clone()) {
            out.push(CallSignal::Offer { call_id: id, opaque: op });
        }
    }
    if let Some(a) = &cm.answer {
        if let (Some(id), Some(op)) = (a.id, a.opaque.clone()) {
            out.push(CallSignal::Answer { call_id: id, opaque: op });
        }
    }
    for ice in &cm.ice_update {
        if let (Some(id), Some(op)) = (ice.id, ice.opaque.clone()) {
            out.push(CallSignal::Ice { call_id: id, opaque: op });
        }
    }
    if let Some(h) = &cm.hangup {
        if let Some(id) = h.id {
            out.push(CallSignal::Hangup { call_id: id, kind: hangup_kind(h.r#type) });
        }
    }
    if let Some(b) = &cm.busy {
        if let Some(id) = b.id {
            out.push(CallSignal::Busy { call_id: id });
        }
    }
    out
}

fn hangup_type(kind: HangupKind) -> i32 {
    match kind {
        HangupKind::Normal => HANGUP_NORMAL,
        HangupKind::Accepted => HANGUP_ACCEPTED,
        HangupKind::Declined => HANGUP_DECLINED,
        HangupKind::Busy => HANGUP_BUSY,
    }
}

fn hangup_kind(t: Option<i32>) -> HangupKind {
    match t {
        Some(HANGUP_ACCEPTED) => HangupKind::Accepted,
        Some(HANGUP_DECLINED) => HangupKind::Declined,
        Some(HANGUP_BUSY) => HangupKind::Busy,
        _ => HangupKind::Normal,
    }
}

/// One active 1:1 call: the [`SignalCall`] driver, its UDP socket, and the host
/// audio handles (opened lazily once media connects).
struct ActiveCall {
    call: SignalCall,
    sock: UdpSocket,
    peer_aci: String,
    cap: Option<u32>,
    trk: Option<u32>,
    track_started: bool,
    /// The route (`true`=speaker) the open output track was created with — so a
    /// mid-call toggle (wandr:audio-focus/controls) can re-open it on the new device.
    cur_speaker: bool,
    mic_buf: Vec<f32>,
    play_buf: Vec<f32>, // stereo playout FIFO; carries samples the host ring couldn't take this tick
    last_state: CallState,
    // Call-log metadata (→ a history entry when the call ends).
    outgoing: bool,
    connected: bool, // reached Connected (answered)
    accepted: bool,  // we pressed accept on an incoming ring
    // ringrtc: as the answerer we must send `accepted` over RTP-data (resent ~1 Hz)
    // or the caller never leaves "ConnectingBeforeAccepted" and streams no audio.
    last_accepted_sent: Option<Instant>,
    declined: bool,  // we declined an incoming ring (hung up while ringing)
    busy: bool,      // peer replied Busy
    // Media-flow diagnostics (→ periodic log; proves where audio stops).
    udp_tx: u64, udp_rx: u64, // datagrams sent / received on the media socket
    aud_tx: u64, aud_rx: u64, // PCM frames mic→peer / peer→speaker
    aud_peak: f32,            // max |sample| of decoded audio (0 ⇒ silent decode)
    mic_peak: f32,            // max |sample| captured from mic (0 ⇒ silent/muted capture)
    wr_ok: u64, wr_zero: u64, // write_pcm_f32 accepted (>0) vs rejected (ring full)
}

/// A snapshot of one call's media-flow counters, for periodic logging.
pub struct MediaStats {
    pub state: CallState,
    pub udp_tx: u64,
    pub udp_rx: u64,
    pub aud_tx: u64,
    pub aud_rx: u64,
    pub aud_peak: f32,
    pub mic_peak: f32,
    pub wr_ok: u64,
    pub wr_zero: u64,
    // Inbound SRTP: datagrams seen as media, decoded OK, decode errored.
    pub srtp_seen: u64,
    pub decode_ok: u64,
    pub decode_err: u64,
    // DIAG inbound RTP: cumulative seq gaps (loss), last ts step (frame samples:
    // 960=20ms/2880=60ms), last payload size.
    pub rtp_seq_gaps: u64,
    pub rtp_ts_step: u32,
    pub rtp_payload_len: usize,
    // Peer's RTP demux ids (what ringrtc sends with → what it expects from us).
    pub peer_pt: u8,
    pub peer_ssrc: u32,
    // DIAG decode failure: the opus decoder's reason for the last failed decode,
    // that packet's TOC byte, and how many failures carried the stereo bit.
    pub dec_err_msg: &'static str,
    pub dec_err_toc: u8,
    pub dec_err_stereo: u64,
}

impl ActiveCall {
    fn new(call: SignalCall, sock: UdpSocket, peer_aci: String, outgoing: bool) -> Self {
        let last_state = call.state();
        Self {
            call, sock, peer_aci, cap: None, trk: None, track_started: false, cur_speaker: false,
            mic_buf: Vec::new(), play_buf: Vec::new(), last_state,
            outgoing, connected: false, accepted: false, last_accepted_sent: None,
            declined: false, busy: false,
            udp_tx: 0, udp_rx: 0, aud_tx: 0, aud_rx: 0, mic_peak: 0.0,
            aud_peak: 0.0, wr_ok: 0, wr_zero: 0,
        }
    }

    /// The call-log outcome for this (now ending) call.
    fn outcome(&self) -> EndedCall {
        EndedCall {
            peer_aci: self.peer_aci.clone(),
            outgoing: self.outgoing,
            connected: self.connected,
            declined: self.declined,
            busy: self.busy,
        }
    }

    /// Pump the host audio once media is connected: mic → `send_audio`, and
    /// `recv_audio` → speaker. Capture (mono) + playback (stereo — this device's
    /// MMAP output is stereo-only) are opened lazily on the first connected tick.
    fn pump_audio(&mut self) {
        // Route switch (earpiece↔loudspeaker): the output route binds at
        // `create_track` (the host reads its comms-route pin then). The user can
        // toggle it mid-call via wandr:audio-focus/controls, so poll the current route
        // (cheap host-side atomic read) and, if it changed, close the output track —
        // the open-block below re-opens it on the new device next. `cur_speaker`
        // avoids a close/re-open every tick.
        let want_speaker = matches!(controls::get_route(), controls::AudioRoute::Speaker);
        if self.trk.is_some() && want_speaker != self.cur_speaker {
            if let Some(t) = self.trk.take() { audio::close(t); }
            self.track_started = false;
            self.play_buf.clear();
            self.cur_speaker = want_speaker;
        }
        // P1 — full-duplex. ORDER MATTERS on this device (taimen): the output
        // USAGE_MEDIA stream falls back to the legacy SHARED path (MMAP -19), but
        // the mic capture takes an MMAP input endpoint — and you can't acquire an
        // MMAP capture endpoint while an output MMAP is held, nor vice-versa.
        // Opening the OUTPUT FIRST (legacy) lets the capture MMAP coexist; opening
        // capture first -889's the output (kills RX). So: output, THEN capture
        // (gated on the output being up).
        if self.trk.is_none() {
            // Stereo on the USAGE_MEDIA path (the only output AAudio can open on
            // this device — voice-comm usage gets -889). The mono Opus frames are
            // duplicated L/R below. voice-call class → the host routes to the
            // arbiter's call route (earpiece by default, speaker on `audio-route`).
            let cfg = TrackConfig { sample_rate: SAMPLE_RATE, channel_layout: ChannelLayout::Stereo, format: Format::PcmF32, class: StreamClass::VoiceCall };
            let h = audio::create_track(cfg);
            if h != 0 {
                self.trk = Some(h); // started after the first write (write-then-start)
                self.cur_speaker = want_speaker; // the route this track opened with
            }
        }
        if self.trk.is_some() && self.cap.is_none() {
            // Capture only after the output is up (see ORDER note above).
            let cfg = TrackConfig { sample_rate: SAMPLE_RATE, channel_layout: ChannelLayout::Mono, format: Format::PcmF32, class: StreamClass::VoiceCall };
            let h = audio::open_capture(cfg);
            if h != 0 {
                audio::start(h);
                self.cap = Some(h);
            }
        }
        if let Some(cap) = self.cap {
            // Drain ALL captured audio each tick, not one frame: the engine ticks at
            // ~35/s (>20 ms apart), but the mic produces 50×20 ms frames/s. A single
            // FRAME read per tick falls behind real-time, the host capture buffer fills
            // and drops the surplus → we send ~26% less than real-time and the far
            // side's jitter buffer starves (interruptions/pops). Loop until the
            // capture ring/buffer is empty (a short read = drained).
            loop {
                let chunk = audio::read_pcm_f32(cap, FRAME as u32);
                let n = chunk.len();
                if n == 0 { break; }
                for &s in &chunk { let a = s.abs(); if a > self.mic_peak { self.mic_peak = a; } }
                self.mic_buf.extend_from_slice(&chunk);
                if n < FRAME { break; } // fewer than requested ⇒ buffer drained
            }
            while self.mic_buf.len() >= FRAME {
                let frame: Vec<f32> = self.mic_buf.drain(..FRAME).collect();
                if self.call.send_audio(&frame).is_ok() {
                    self.aud_tx += 1;
                }
            }
        }
        if let Some(trk) = self.trk {
            // 1. Drain every decoded frame into the stereo playout FIFO (mono Opus
            //    → L/R). The host ring is tiny (~32 ms); writing each frame straight
            //    through dropped most of a jittery burst. Buffer here instead.
            while let Some(pcm) = self.call.recv_audio() {
                for &s in &pcm {
                    let a = s.abs();
                    if a > self.aud_peak { self.aud_peak = a; }
                }
                self.play_buf.extend(pcm.iter().flat_map(|&s| [s, s]));
                self.aud_rx += 1;
            }
            // Cap playout latency: a sustained stall must not grow the FIFO without
            // bound. 200 ms of stereo @ 48 k = 19_200 samples; drop the oldest past
            // that (the host ring's underrun-resync covers the resulting gap).
            const MAX_PLAY_SAMPLES: usize = 48_000 * 2 / 5;
            if self.play_buf.len() > MAX_PLAY_SAMPLES {
                let drop = self.play_buf.len() - MAX_PLAY_SAMPLES;
                self.play_buf.drain(..drop);
            }
            // 2. Feed the host ring as much as it has room for; write_pcm_f32 returns
            //    the stereo frames it accepted (2 samples each). Keep the rest for
            //    the next tick rather than dropping it.
            if !self.play_buf.is_empty() {
                let frames = audio::write_pcm_f32(trk, &self.play_buf) as usize;
                let consumed = (frames * 2).min(self.play_buf.len());
                if consumed > 0 {
                    self.play_buf.drain(..consumed);
                    self.wr_ok += 1;
                    if !self.track_started {
                        audio::start(trk); // write-then-start: prime before the HAL pulls
                        self.track_started = true;
                    }
                } else {
                    self.wr_zero += 1; // ring full this tick — remainder carried forward
                }
            }
        }
    }

    fn teardown_audio(&mut self) {
        if let Some(c) = self.cap.take() {
            audio::close(c);
        }
        if let Some(t) = self.trk.take() {
            audio::close(t);
        }
    }
}

/// Summary of a call that just ended, for the conversation call-log entry.
pub struct EndedCall {
    pub peer_aci: String,
    pub outgoing: bool,
    pub connected: bool,
    pub declined: bool,
    pub busy: bool,
}

/// The engine-side call orchestrator: at most one active 1:1 call.
pub struct CallEngine {
    active: Option<ActiveCall>,
    /// Set when a call ends — drained by the engine to log a history entry.
    ended: Option<EndedCall>,
}

impl Default for CallEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CallEngine {
    pub fn new() -> Self {
        Self { active: None, ended: None }
    }

    /// Take the just-ended call's summary (for a conversation call-log entry).
    pub fn take_ended(&mut self) -> Option<EndedCall> {
        self.ended.take()
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// Media-flow counters for the active call (diagnostic), or `None` (idle).
    pub fn media_stats(&self) -> Option<MediaStats> {
        self.active.as_ref().map(|a| {
            let (srtp_seen, decode_ok, decode_err) = a.call.media_diag();
            let (rtp_seq_gaps, rtp_ts_step, rtp_payload_len) = a.call.rtp_diag();
            let (peer_pt, peer_ssrc) = a.call.rtp_peer_ids();
            let (dec_err_msg, dec_err_toc, dec_err_stereo) = a.call.dec_err_diag();
            MediaStats {
                state: a.call.state(),
                udp_tx: a.udp_tx,
                udp_rx: a.udp_rx,
                aud_tx: a.aud_tx,
                aud_rx: a.aud_rx,
                aud_peak: a.aud_peak,
                mic_peak: a.mic_peak,
                wr_ok: a.wr_ok,
                wr_zero: a.wr_zero,
                srtp_seen,
                decode_ok,
                decode_err,
                rtp_seq_gaps,
                rtp_ts_step,
                rtp_payload_len,
                peer_pt,
                peer_ssrc,
                dec_err_msg,
                dec_err_toc,
                dec_err_stereo,
            }
        })
    }

    pub fn peer(&self) -> String {
        self.active.as_ref().map(|a| a.peer_aci.clone()).unwrap_or_default()
    }

    /// The driver state of the active call, or `None` (idle).
    pub fn state(&self) -> Option<CallState> {
        self.active.as_ref().map(|a| a.call.state())
    }

    /// The `call_id` of the active call, if any.
    pub fn active_call_id(&self) -> Option<u64> {
        self.active.as_ref().map(|a| a.call.call_id())
    }

    /// DIAG: one-line ICE/keying snapshot for the active call (role/state/pair).
    pub fn conn_debug(&self) -> Option<String> {
        self.active.as_ref().map(|a| a.call.conn_debug())
    }

    /// Place an outgoing call. `my_identity`/`peer_identity` are the serialized
    /// identity public keys that bind the SRTP keys (caller = us = offerer first).
    /// Returns the `CallSignal`s to send (the `Offer` + trickled ICE).
    pub fn place(
        &mut self,
        call_id: u64,
        my_identity: Vec<u8>,
        peer_identity: Vec<u8>,
        peer_aci: String,
        turn: Option<wandr_call::turn::TurnConfig>,
    ) -> Result<Vec<CallSignal>, String> {
        if self.active.is_some() {
            return Err("a call is already active".into());
        }
        let sock = bind_socket()?;
        let local = local_candidate_addr(&sock)?;
        // caller = us (offerer): caller_identity = my, callee_identity = peer.
        let call = SignalCall::place(call_id, local, my_identity, peer_identity, turn)
            .map_err(|e| e.to_string())?;
        let mut active = ActiveCall::new(call, sock, peer_aci, true);
        let sigs = active.call.poll_signals();
        self.active = Some(active);
        Ok(sigs)
    }

    /// Handle an inbound `CallMessage` from `sender_aci`. Starts ringing on a fresh
    /// `Offer`; otherwise feeds signaling to the active call. Returns
    /// `(signals_to_send, incoming_peer)` — `incoming_peer` is `Some(aci)` when a
    /// new call started ringing (raise the incoming-call UI), and the signals are a
    /// `Busy` reply when an offer arrives while already on another call.
    pub fn on_call_message(
        &mut self,
        cm: &CallMessage,
        sender_aci: &str,
        my_identity: &[u8],
        sender_identity: &[u8],
        my_device_id: u32,
        turn: Option<wandr_call::turn::TurnConfig>,
    ) -> (Vec<CallSignal>, Option<String>) {
        let signals = call_message_to_signals(cm);
        // Multi-device coordination: when THIS device (e.g. wandr) accepts/declines,
        // the caller relays a `Hangup{type=Accepted/Declined/Busy, device_id=ours}`
        // to the user's *other* devices so they stop ringing. That message is also
        // delivered to us — but it's the echo of our OWN action, NOT a signal to end
        // our just-accepted call. ringrtc ignores a hangup whose device_id is its
        // own; without this, wandr (device_id N) hangs up on itself the instant it
        // answers. Only a `Normal` hangup, or a coordination hangup from a DIFFERENT
        // device, ends our call.
        let self_coordination_hangup = matches!(
            cm.hangup.as_ref().and_then(|h| h.r#type),
            Some(HANGUP_ACCEPTED) | Some(HANGUP_DECLINED) | Some(HANGUP_BUSY)
        ) && cm.hangup.as_ref().and_then(|h| h.device_id) == Some(my_device_id);
        let mut out = Vec::new();
        let mut incoming = None;
        for sig in signals {
            // Drop our own action echoed back (see above) so it can't end the call.
            if self_coordination_hangup && matches!(sig, CallSignal::Hangup { .. }) {
                continue;
            }
            match (&sig, self.active.is_some()) {
                // A fresh incoming offer with no active call → start ringing.
                // caller = the remote offerer (sender), callee = us.
                (CallSignal::Offer { call_id, opaque }, false) => {
                    let (call_id, opaque) = (*call_id, opaque.clone());
                    match self.start_incoming(call_id, sender_aci, sender_identity, my_identity, &opaque, turn.clone()) {
                        Ok(()) => incoming = Some(sender_aci.to_owned()),
                        Err(_) => out.push(CallSignal::Busy { call_id }),
                    }
                }
                // An offer for a different call while busy → Busy.
                (CallSignal::Offer { call_id, .. }, true) => {
                    if self.active_call_id() != Some(*call_id) {
                        out.push(CallSignal::Busy { call_id: *call_id });
                    }
                }
                // Everything else feeds the active call.
                _ => {
                    if let Some(a) = &mut self.active {
                        if matches!(sig, CallSignal::Busy { .. }) {
                            a.busy = true;
                        }
                        let _ = a.call.on_signal(sig);
                    }
                }
            }
        }
        (out, incoming)
    }

    fn start_incoming(
        &mut self,
        call_id: u64,
        caller_aci: &str,
        caller_identity: &[u8],
        callee_identity: &[u8],
        offer_opaque: &[u8],
        turn: Option<wandr_call::turn::TurnConfig>,
    ) -> Result<(), String> {
        if caller_identity.is_empty() || callee_identity.is_empty() {
            return Err("missing identity key for the call".into());
        }
        let sock = bind_socket()?;
        let local = local_candidate_addr(&sock)?;
        let call = SignalCall::incoming(
            call_id,
            local,
            caller_identity.to_vec(),
            callee_identity.to_vec(),
            offer_opaque,
            turn,
        )
        .map_err(|e| e.to_string())?;
        self.active = Some(ActiveCall::new(call, sock, caller_aci.to_owned(), false));
        Ok(())
    }

    /// Accept the ringing call; returns the `Answer` + trickled ICE to send.
    pub fn accept(&mut self) -> Vec<CallSignal> {
        if let Some(a) = &mut self.active {
            a.accepted = true; // pressed green → not a decline, even if media never connects
            let _ = a.call.accept();
            return a.call.poll_signals();
        }
        Vec::new()
    }

    /// Hang up; returns the `Hangup` to send and drops the call.
    pub fn hangup(&mut self) -> Vec<CallSignal> {
        if let Some(mut a) = self.active.take() {
            // Declined = we hung up an incoming ring we never accepted. If we
            // pressed accept (`accepted`) but media never connected, it's NOT a
            // decline — it falls through to `in-missed` (an answered call that
            // failed to connect), not `in-declined`.
            if !a.outgoing && !a.connected && !a.accepted {
                a.declined = true;
            }
            a.call.hangup();
            let sigs = a.call.poll_signals();
            self.ended = Some(a.outcome());
            a.teardown_audio();
            return sigs;
        }
        Vec::new()
    }

    /// Pump UDP + audio one tick. Returns `(signals_to_send, new_state)` where
    /// `new_state` is `Some` only when the driver state changed this tick. Drops
    /// the call (and its audio) when it reaches `Ended`.
    pub fn tick(&mut self, now: Instant) -> (Vec<CallSignal>, Option<CallState>) {
        let (sigs, new_state, changed) = {
            let Some(a) = &mut self.active else { return (Vec::new(), None) };
            let mut buf = [0u8; 2048];
            a.call.handle_timeout(now);
            for (dst, dg) in a.call.poll_transmit() {
                if a.sock.send_to(&dg, dst).is_ok() {
                    a.udp_tx += 1;
                }
            }
            while let Ok((n, src)) = a.sock.recv_from(&mut buf) {
                a.udp_rx += 1;
                let _ = a.call.handle_datagram(src, &buf[..n]);
            }
            if a.call.is_connected() {
                a.connected = true;
                // ringrtc: the answerer must send `accepted` over RTP-data so the
                // caller leaves "ConnectingBeforeAccepted" and starts streaming.
                // Send on connect + resend ~1 Hz (ringrtc resends too) until ended.
                if a.accepted && a.call.is_answerer() {
                    let due = a
                        .last_accepted_sent
                        .map(|t| now.duration_since(t) >= std::time::Duration::from_secs(1))
                        .unwrap_or(true);
                    if due && a.call.send_accepted().is_ok() {
                        a.last_accepted_sent = Some(now);
                    }
                }
                a.pump_audio();
            }
            let sigs = a.call.poll_signals();
            let new_state = a.call.state();
            let changed = new_state != a.last_state;
            a.last_state = new_state;
            (sigs, new_state, changed)
        };
        if new_state == CallState::Ended {
            if let Some(mut a) = self.active.take() {
                self.ended = Some(a.outcome()); // remote-ended → log it
                a.teardown_audio();
            }
        }
        (sigs, if changed { Some(new_state) } else { None })
    }
}

/// A non-blocking UDP socket bound to an ephemeral port (the call's media socket).
fn bind_socket() -> Result<UdpSocket, String> {
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("bind udp: {e}"))?;
    sock.set_nonblocking(true).map_err(|e| format!("nonblock: {e}"))?;
    Ok(sock)
}

/// Our host ICE candidate address: the LAN IP (reachable by a peer on the same
/// network) paired with the bound port. Falls back to loopback (same-device test).
fn local_candidate_addr(sock: &UdpSocket) -> Result<SocketAddr, String> {
    let port = sock.local_addr().map_err(|e| format!("local_addr: {e}"))?.port();
    let ip = local_lan_ip().unwrap_or_else(|| std::net::Ipv4Addr::LOCALHOST.into());
    Ok(SocketAddr::new(ip, port))
}
