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
//! **Interim identity binding (Phase 3 must replace):** ringrtc's DH-SRTP HKDF
//! binds the SRTP keys to the two participants' *identity public keys*. Here we
//! bind to their **ACIs** instead — both wart ends compute the same `info`, so
//! wart↔wart calls key correctly, but this will NOT interop with a real Signal
//! client until we feed the real serialized identity keys (the open noted in
//! `tasks/75`). See [`identity_for`].

use std::net::{SocketAddr, UdpSocket};
use std::time::Instant;

use libsignal_service::proto::{call_message, CallMessage};
use uuid::Uuid;
use wart_call::local_lan_ip;
use wart_call::signal::{CallSignal, CallState, HangupKind, SignalCall};

use crate::my::skiko_gfx::audio::{self, ChannelLayout, Format, TrackConfig};

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

/// The SRTP-HKDF identity binding for an ACI (interim — see module docs). Uses the
/// ACI's 16 raw UUID bytes so both ends derive an identical value deterministically.
pub fn identity_for(aci: &str) -> Vec<u8> {
    Uuid::parse_str(aci)
        .map(|u| u.as_bytes().to_vec())
        .unwrap_or_else(|_| aci.as_bytes().to_vec())
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
    last_state: CallState,
}

impl ActiveCall {
    fn new(call: SignalCall, sock: UdpSocket, peer_aci: String) -> Self {
        let last_state = call.state();
        Self { call, sock, peer_aci, cap: None, trk: None, track_started: false, mic_buf: Vec::new(), last_state }
    }

    /// Pump the host audio once media is connected: mic → `send_audio`, and
    /// `recv_audio` → speaker. Capture (mono) + playback (stereo — this device's
    /// MMAP output is stereo-only) are opened lazily on the first connected tick.
    fn pump_audio(&mut self) {
        if self.cap.is_none() {
            let cfg = TrackConfig { sample_rate: SAMPLE_RATE, channel_layout: ChannelLayout::Mono, format: Format::PcmF32 };
            let h = audio::open_capture(cfg);
            if h != 0 {
                audio::start(h);
                self.cap = Some(h);
            }
        }
        if self.trk.is_none() {
            let cfg = TrackConfig { sample_rate: SAMPLE_RATE, channel_layout: ChannelLayout::Stereo, format: Format::PcmF32 };
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
                let _ = self.call.send_audio(&frame);
            }
        }
        if let Some(trk) = self.trk {
            let mut wrote = false;
            while let Some(pcm) = self.call.recv_audio() {
                let stereo: Vec<f32> = pcm.iter().flat_map(|&s| [s, s]).collect();
                let _ = audio::write_pcm_f32(trk, &stereo);
                wrote = true;
            }
            if wrote && !self.track_started {
                audio::start(trk);
                self.track_started = true;
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

/// The engine-side call orchestrator: at most one active 1:1 call.
pub struct CallEngine {
    active: Option<ActiveCall>,
}

impl Default for CallEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CallEngine {
    pub fn new() -> Self {
        Self { active: None }
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
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

    /// Place an outgoing call. `my_aci`/`peer_aci` bind the SRTP keys (interim).
    /// Returns the `CallSignal`s to send (the `Offer` + trickled ICE).
    pub fn place(
        &mut self,
        call_id: u64,
        my_aci: &str,
        peer_aci: String,
    ) -> Result<Vec<CallSignal>, String> {
        if self.active.is_some() {
            return Err("a call is already active".into());
        }
        let sock = bind_socket()?;
        let local = local_candidate_addr(&sock)?;
        // caller = us (offerer): caller_identity = my, callee_identity = peer.
        let call = SignalCall::place(call_id, local, identity_for(my_aci), identity_for(&peer_aci))
            .map_err(|e| e.to_string())?;
        let mut active = ActiveCall::new(call, sock, peer_aci);
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
        my_aci: &str,
    ) -> (Vec<CallSignal>, Option<String>) {
        let signals = call_message_to_signals(cm);
        let mut out = Vec::new();
        let mut incoming = None;
        for sig in signals {
            match (&sig, self.active.is_some()) {
                // A fresh incoming offer with no active call → start ringing.
                (CallSignal::Offer { call_id, opaque }, false) => {
                    let (call_id, opaque) = (*call_id, opaque.clone());
                    match self.start_incoming(call_id, sender_aci, my_aci, &opaque) {
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
        my_aci: &str,
        offer_opaque: &[u8],
    ) -> Result<(), String> {
        let sock = bind_socket()?;
        let local = local_candidate_addr(&sock)?;
        // caller = the remote offerer; callee = us.
        let call = SignalCall::incoming(
            call_id,
            local,
            identity_for(caller_aci),
            identity_for(my_aci),
            offer_opaque,
        )
        .map_err(|e| e.to_string())?;
        self.active = Some(ActiveCall::new(call, sock, caller_aci.to_owned()));
        Ok(())
    }

    /// Accept the ringing call; returns the `Answer` + trickled ICE to send.
    pub fn accept(&mut self) -> Vec<CallSignal> {
        if let Some(a) = &mut self.active {
            let _ = a.call.accept();
            return a.call.poll_signals();
        }
        Vec::new()
    }

    /// Hang up; returns the `Hangup` to send and drops the call.
    pub fn hangup(&mut self) -> Vec<CallSignal> {
        if let Some(mut a) = self.active.take() {
            a.call.hangup();
            let sigs = a.call.poll_signals();
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
                let _ = a.sock.send_to(&dg, dst);
            }
            while let Ok((n, src)) = a.sock.recv_from(&mut buf) {
                let _ = a.call.handle_datagram(src, &buf[..n]);
            }
            if a.call.is_connected() {
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
