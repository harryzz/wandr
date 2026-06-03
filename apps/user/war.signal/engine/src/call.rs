//! call.rs — the engine's Signal 1:1 voice-call adapter (Phase 2b-ii).
//!
//! Bridges libsignal's `CallMessage` (the signaling envelope that rides the
//! existing E2E channel) and the host audio/UDP to the `wart_call` [`SignalCall`]
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
use wart_call::local_lan_ip;
use wart_call::signal::{CallSignal, CallState, HangupKind, SignalCall};
use wart_call::turn::TurnConfig;

use crate::my::skiko_gfx::audio::{self, ChannelLayout, Format, StreamClass, TrackConfig};

const SAMPLE_RATE: u32 = wart_call::SAMPLE_RATE;
/// 20 ms @ 48 kHz mono — the Opus frame + audio tick granularity.
const FRAME: usize = SAMPLE_RATE as usize / 1000 * 20;

// Proto enum ints (SignalService.proto `CallMessage`), stable on the wire — used
// directly to avoid depending on prost's generated enum variant names.
const OFFER_AUDIO_CALL: i32 = 0;
const HANGUP_NORMAL: i32 = 0;
const HANGUP_ACCEPTED: i32 = 1;
const HANGUP_DECLINED: i32 = 2;
const HANGUP_BUSY: i32 = 3;

/// Build a wart-call [`TurnConfig`] from a Signal calling relay. Prefers the
/// IP-pinned URLs (`urlsWithIps`) and requires `ip:port` form — the wasip2 guest has
/// no reliable DNS, and wart-call demuxes inbound TURN traffic by the server address.
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
    mic_buf: Vec<f32>,
    play_buf: Vec<f32>, // stereo playout FIFO; carries samples the host ring couldn't take this tick
    last_state: CallState,
    // Call-log metadata (→ a history entry when the call ends).
    outgoing: bool,
    connected: bool, // reached Connected (answered)
    accepted: bool,  // we pressed accept on an incoming ring
    declined: bool,  // we declined an incoming ring (hung up while ringing)
    busy: bool,      // peer replied Busy
    // Media-flow diagnostics (→ periodic log; proves where audio stops).
    udp_tx: u64, udp_rx: u64, // datagrams sent / received on the media socket
    aud_tx: u64, aud_rx: u64, // PCM frames mic→peer / peer→speaker
    aud_peak: f32,            // max |sample| of decoded audio (0 ⇒ silent decode)
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
}

impl ActiveCall {
    fn new(call: SignalCall, sock: UdpSocket, peer_aci: String, outgoing: bool) -> Self {
        let last_state = call.state();
        Self {
            call, sock, peer_aci, cap: None, trk: None, track_started: false,
            mic_buf: Vec::new(), play_buf: Vec::new(), last_state,
            outgoing, connected: false, accepted: false, declined: false, busy: false,
            udp_tx: 0, udp_rx: 0, aud_tx: 0, aud_rx: 0,
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
        // RX_ONLY again (temporary): full-duplex mic is deferred to a proper P1
        // pass. Enabling capture made the AAudioService capture thread spin
        // (processDataNow "wait for valid timestamps") AND the mic didn't actually
        // carry audio to the peer — needs its own investigation (capture config /
        // in+out coexistence). Keep receive-only (real voice, low CPU) for now.
        const RX_ONLY: bool = true;
        if !RX_ONLY && self.cap.is_none() {
            let cfg = TrackConfig { sample_rate: SAMPLE_RATE, channel_layout: ChannelLayout::Mono, format: Format::PcmF32, class: StreamClass::VoiceCall };
            let h = audio::open_capture(cfg);
            if h != 0 {
                audio::start(h);
                self.cap = Some(h);
            }
        }
        if self.trk.is_none() {
            // Stereo on the USAGE_MEDIA path (the only output AAudio can open on
            // this device — voice-comm usage gets -889). The mono Opus frames are
            // duplicated L/R below. Ducking is avoided by staying in NORMAL mode
            // (COMMS_MODE=false), not by the stream usage.
            // voice-call class → the host routes to the arbiter's call route
            // (earpiece by default, speaker on `audio-route`).
            let cfg = TrackConfig { sample_rate: SAMPLE_RATE, channel_layout: ChannelLayout::Stereo, format: Format::PcmF32, class: StreamClass::VoiceCall };
            let h = audio::create_track(cfg);
            if h != 0 {
                self.trk = Some(h); // started after the first write (write-then-start)
            }
        }
        if let Some(cap) = self.cap {
            let chunk = audio::read_pcm_f32(cap, FRAME as u32);
            if !chunk.is_empty() {
                self.mic_buf.extend_from_slice(&chunk);
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
            MediaStats {
                state: a.call.state(),
                udp_tx: a.udp_tx,
                udp_rx: a.udp_rx,
                aud_tx: a.aud_tx,
                aud_rx: a.aud_rx,
                aud_peak: a.aud_peak,
                wr_ok: a.wr_ok,
                wr_zero: a.wr_zero,
                srtp_seen,
                decode_ok,
                decode_err,
                rtp_seq_gaps,
                rtp_ts_step,
                rtp_payload_len,
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

    /// Place an outgoing call. `my_identity`/`peer_identity` are the serialized
    /// identity public keys that bind the SRTP keys (caller = us = offerer first).
    /// Returns the `CallSignal`s to send (the `Offer` + trickled ICE).
    pub fn place(
        &mut self,
        call_id: u64,
        my_identity: Vec<u8>,
        peer_identity: Vec<u8>,
        peer_aci: String,
        turn: Option<wart_call::turn::TurnConfig>,
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
        turn: Option<wart_call::turn::TurnConfig>,
    ) -> (Vec<CallSignal>, Option<String>) {
        let signals = call_message_to_signals(cm);
        let mut out = Vec::new();
        let mut incoming = None;
        for sig in signals {
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
        turn: Option<wart_call::turn::TurnConfig>,
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
