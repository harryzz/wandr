//! [`PeerSession`] — the call engine API. Composes signaling + transport + media
//! into an event-loop-driveable session over real UDP datagrams.
//!
//! ```ignore
//! let sock = UdpSocket::bind("0.0.0.0:0")?;        // wasi:sockets in the guest
//! sock.set_nonblocking(true)?;
//! let mut s = PeerSession::new(Role::Offerer, sock.local_addr()?)?;
//! let offer = s.local_signaling().to_sdp();        // → send over signaling channel
//! s.set_remote_signaling(&Signaling::from_sdp(&answer)?)?;   // ← peer's answer
//! loop {
//!     for (dest, data) in s.poll_transmit() { sock.send_to(&data, dest); }
//!     while let Ok((n, src)) = sock.recv_from(&mut buf) { s.handle_datagram(src, &buf[..n])?; }
//!     s.handle_timeout(now());
//!     if s.is_connected() {
//!         s.send_audio(&mic_frame)?;                          // mic → wire
//!         while let Some(pcm) = s.recv_audio() { speaker.play(&pcm); }  // wire → speaker
//!     }
//! }
//! ```

use std::net::SocketAddr;
use std::time::Instant;

use crate::media::MediaSession;
use crate::signaling::Signaling;
use crate::transport::{Demux, Transport};
use crate::video::{VideoDiag, VideoFrame, VP8_PAYLOAD_TYPE};
use crate::{Error, OPUS_PAYLOAD_TYPE, SAMPLE_RATE};

/// Which end of the call this is. Offerer sends the SDP offer; Answerer answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Offerer,
    Answerer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    New,
    Connecting,
    Connected,
}

pub struct PeerSession {
    role: Role,
    local_addr: SocketAddr,
    ufrag: String,
    pwd: String,
    transport: Transport,
    media: Option<MediaSession>,
    /// The offer's media direction (answerer mirrors its reverse).
    remote_direction: Option<String>,
    /// Decoded PCM frames from inbound media (drained by `recv_audio`).
    audio_in: Vec<Vec<f32>>,
    /// Inbound-media diagnostics: datagrams demuxed as SRTP media, and how many
    /// then decrypt+decode OK vs error. `seen` climbing with `err` climbing (and
    /// `ok` flat) = SRTP decrypt mismatch with the peer (key/KDF convention).
    media_seen: u64,
    decode_ok: u64,
    decode_err: u64,
    /// Negotiated Opus payload type. Defaults to [`OPUS_PAYLOAD_TYPE`]; on the
    /// WebRTC path `set_remote_signaling` adopts the peer's SDP rtpmap PT, so we
    /// send + decode on whatever the peer uses (ringrtc fixes it at 102).
    audio_pt: u8,
    /// Whole reassembled inbound video frames (drained by `recv_video`).
    video_in: Vec<VideoFrame>,
    /// The peer asked for a keyframe (RTCP PLI/FIR received) — drained by
    /// `take_keyframe_request`; the guest answers with encoder `request-keyframe`.
    keyframe_requested: bool,
    /// We owe the peer a PLI (inbound loss detected, or the guest's decoder asked
    /// via `request_keyframe`). Sent rate-limited from `handle_timeout`.
    pli_wanted: bool,
    last_pli_tx: Option<Instant>,
    last_sr_tx: Option<Instant>,
    /// The peer's latest REMB bandwidth estimate (bps), if it sends one — the
    /// guest can feed this to the encoder's `set-bitrate` (v1 congestion control).
    peer_remb_bps: Option<u32>,
    /// The peer's last RTCP sender report `(ssrc, ntp_time, rtp_time)` — the
    /// wall-clock ⇄ RTP-timestamp anchor a renderer needs for A/V sync.
    peer_sr: Option<(u32, u64, u32)>,
    /// RTCP counters: PLIs we sent / PLIs+FIRs the peer sent us / SRTCP packets
    /// that failed to unprotect.
    pli_tx: u64,
    pli_rx: u64,
    rtcp_err: u64,
    /// Monotonic seqnum for the ringrtc RTP-data control channel (the `accepted`
    /// message). Incremented per send; ringrtc resends accumulated state ~1 Hz.
    #[cfg(feature = "signal")]
    rtp_data_seqnum: u64,
    /// Accumulated outbound RTP-data state (ringrtc semantics — task 93 Phase 5):
    /// the call id once known, `accepted` while the answerer is engaged, and our
    /// camera on/off. Resent ~1 Hz from `handle_timeout` once keyed.
    #[cfg(feature = "signal")]
    rtp_call_id: Option<u64>,
    #[cfg(feature = "signal")]
    accepted_engaged: bool,
    #[cfg(feature = "signal")]
    video_enabled_local: Option<bool>,
    #[cfg(feature = "signal")]
    last_rtp_data_tx: Option<Instant>,
    /// Peer state decoded from inbound RTP-data: camera on/off (+ a
    /// clear-on-read toggle event), requested send bitrate, rtp-data hangup.
    #[cfg(feature = "signal")]
    peer_video_enabled: Option<bool>,
    #[cfg(feature = "signal")]
    peer_video_toggle: Option<bool>,
    #[cfg(feature = "signal")]
    peer_max_bitrate: Option<u64>,
    #[cfg(feature = "signal")]
    peer_rtp_hangup: bool,
    /// Outgoing CVO rotation (camera sensor orientation), applied to the video
    /// track when media exists (pending until then).
    video_rotation: Option<u32>,
    /// Our receive budget (bps) — advertised to the peer via RTCP REMB (and,
    /// on the Signal path, rtp_data receiverStatus) so its sender-side BWE
    /// ramps up (it gets no TWCC/RR from us). `None` = don't advertise.
    receive_budget_bps: Option<u32>,
    last_remb_tx: Option<Instant>,
    /// Optional host AEAD backend for the SRTP GCM (feature `host-aead`). Set by the
    /// guest via [`PeerSession::set_aead_provider`] before media starts; `None` =
    /// the in-wasm software AES. Used when `ensure_media` builds the `MediaSession`.
    #[cfg(feature = "host-aead")]
    aead_provider: Option<Box<dyn crate::AeadProvider>>,
}

impl PeerSession {
    /// `local_addr` is the bound UDP socket address — advertised as our host
    /// candidate and used as the source address of our datagrams.
    pub fn new(role: Role, local_addr: SocketAddr) -> Result<Self, Error> {
        // Per-role ICE creds (exchanged via signaling). Production generates these
        // randomly; fixed-per-role is fine for one call pair.
        let (ufrag, pwd) = match role {
            Role::Offerer => ("ufrAgentA", "passwordApasswordApasswordA00"),
            Role::Answerer => ("ufrAgentB", "passwordBpasswordBpasswordB00"),
        };
        let transport = Transport::new(role, ufrag, pwd, local_addr)?;
        Ok(Self {
            role,
            local_addr,
            ufrag: ufrag.to_owned(),
            pwd: pwd.to_owned(),
            transport,
            media: None,
            remote_direction: None,
            audio_in: Vec::new(),
            media_seen: 0, decode_ok: 0, decode_err: 0,
            audio_pt: OPUS_PAYLOAD_TYPE,
            video_in: Vec::new(),
            keyframe_requested: false,
            pli_wanted: false,
            last_pli_tx: None,
            last_sr_tx: None,
            peer_remb_bps: None,
            peer_sr: None,
            pli_tx: 0, pli_rx: 0, rtcp_err: 0,
            #[cfg(feature = "signal")]
            rtp_data_seqnum: 0,
            #[cfg(feature = "signal")]
            rtp_call_id: None,
            #[cfg(feature = "signal")]
            accepted_engaged: false,
            #[cfg(feature = "signal")]
            video_enabled_local: None,
            #[cfg(feature = "signal")]
            last_rtp_data_tx: None,
            #[cfg(feature = "signal")]
            peer_video_enabled: None,
            #[cfg(feature = "signal")]
            peer_video_toggle: None,
            #[cfg(feature = "signal")]
            peer_max_bitrate: None,
            #[cfg(feature = "signal")]
            peer_rtp_hangup: false,
            video_rotation: None,
            receive_budget_bps: None,
            last_remb_tx: None,
            #[cfg(feature = "host-aead")]
            aead_provider: None,
        })
    }

    /// A Signal call session (ringrtc V4 keying): no DTLS — SRTP keys come from an
    /// X25519-DH exchange whose public key rides in signaling ([`Signaling::public_key`]).
    /// `caller_identity`/`callee_identity` are the two ACI identity public keys
    /// (serialized, caller = offerer first) that authenticate the derived keys.
    /// Pair the signaling with the [`crate::signal`] `opaque` codec.
    /// `turn` (optional) allocates a TURN relay so the call connects across NAT;
    /// `None` gathers host candidates only.
    #[cfg(feature = "signal")]
    pub fn new_signal(
        role: Role,
        local_addr: SocketAddr,
        caller_identity: Vec<u8>,
        callee_identity: Vec<u8>,
        turn: Option<crate::turn::TurnConfig>,
    ) -> Result<Self, Error> {
        // Random per-call ICE creds (advertised in our offer/answer). libwebrtc
        // (real Signal) requires ufrag ≥4 and pwd ≥22 chars from the ICE charset;
        // ringrtc generates these per call, so fixed creds would be an interop tell.
        let ufrag = random_ice_cred(8);
        let pwd = random_ice_cred(24);
        let transport = Transport::new_signal(
            role, &ufrag, &pwd, local_addr, caller_identity, callee_identity, turn,
        )?;
        Ok(Self {
            role,
            local_addr,
            ufrag,
            pwd,
            transport,
            media: None,
            remote_direction: None,
            audio_in: Vec::new(),
            media_seen: 0, decode_ok: 0, decode_err: 0,
            audio_pt: OPUS_PAYLOAD_TYPE,
            video_in: Vec::new(),
            keyframe_requested: false,
            pli_wanted: false,
            last_pli_tx: None,
            last_sr_tx: None,
            peer_remb_bps: None,
            peer_sr: None,
            pli_tx: 0, pli_rx: 0, rtcp_err: 0,
            rtp_data_seqnum: 0,
            rtp_call_id: None,
            accepted_engaged: false,
            video_enabled_local: None,
            last_rtp_data_tx: None,
            peer_video_enabled: None,
            peer_video_toggle: None,
            peer_max_bitrate: None,
            peer_rtp_hangup: false,
            video_rotation: None,
            receive_budget_bps: None,
            last_remb_tx: None,
            #[cfg(feature = "host-aead")]
            aead_provider: None,
        })
    }

    /// Inject the host AEAD backend for the SRTP GCM (feature `host-aead`). Call
    /// before the first media frame; once `ensure_media` has built the `MediaSession`
    /// the SRTP contexts are fixed. `None` (the default) keeps the in-wasm AES.
    #[cfg(feature = "host-aead")]
    pub fn set_aead_provider(&mut self, provider: Box<dyn crate::AeadProvider>) {
        self.aead_provider = Some(provider);
    }

    /// Our signaling params — marshal to SDP and send over the signaling channel.
    pub fn local_signaling(&self) -> Signaling {
        Signaling {
            ice_ufrag: self.ufrag.clone(),
            ice_pwd: self.pwd.clone(),
            fingerprint: self.transport.fingerprint().to_owned(),
            setup: match self.role {
                Role::Offerer => "actpass".to_owned(),
                Role::Answerer => "active".to_owned(),
            },
            direction: match self.role {
                // Offer sendrecv; an answer mirrors the offer's direction.
                Role::Offerer => "sendrecv".to_owned(),
                Role::Answerer => {
                    let offer = self.remote_direction.as_deref().unwrap_or("sendrecv");
                    crate::signaling::reverse_direction(offer).to_owned()
                }
            },
            candidates: vec![candidate_string(self.local_addr)],
            audio_pt: self.audio_pt,
            // WebRTC-native path keys via DTLS (None); the Signal path advertises
            // its ephemeral X25519 public key here for the peer's DH.
            #[cfg(feature = "signal")]
            public_key: self.transport.signal_public_key().map(|k| k.to_vec()),
            #[cfg(not(feature = "signal"))]
            public_key: None,
        }
    }

    /// Apply the peer's signaling (from their SDP) → start ICE connectivity. The
    /// peer's cert fingerprint is checked against the handshake cert on connect.
    pub fn set_remote_signaling(&mut self, remote: &Signaling) -> Result<(), Error> {
        // The bundled (SDP/DTLS) path carries candidates here; the Signal trickle
        // path carries none (they arrive via `add_remote_candidate`), so an empty
        // set is allowed — ICE just has no pair to check until candidates trickle in.
        let remotes: Vec<SocketAddr> =
            remote.candidates.iter().filter_map(|c| parse_candidate_addr(c)).collect();
        self.remote_direction = Some(remote.direction.clone());
        // Adopt the peer's Opus PT (WebRTC: parsed from their SDP; Signal: 102).
        self.audio_pt = remote.audio_pt;
        self.transport.set_remote(
            &remote.ice_ufrag,
            &remote.ice_pwd,
            &remote.fingerprint,
            remote.public_key.as_deref(),
            &remotes,
        )
    }

    /// Add one trickled remote ICE candidate (a candidate line in
    /// [`Signaling::candidates`] form, i.e. without the `candidate:` prefix). The
    /// Signal path delivers candidates this way, one per `IceUpdate`, after
    /// [`Self::set_remote_signaling`]. Unparseable lines are ignored.
    pub fn add_remote_candidate(&mut self, candidate: &str) -> Result<(), Error> {
        match parse_candidate_addr(candidate) {
            Some(addr) => {
                // A `typ relay` candidate is reachable only by sending through our
                // own relay (its TURN server won't accept our raw host/srflx).
                let is_relay = candidate.contains("typ relay");
                self.transport.add_remote_candidate(addr, is_relay)
            }
            None => Ok(()),
        }
    }

    /// New local candidate lines to trickle to the peer (Signal relay path) — the
    /// relay candidate appears once the TURN allocation completes. Drain each tick
    /// and send each as a `CallSignal::Ice`. Empty without a `TurnConfig`.
    #[cfg(feature = "signal")]
    pub fn take_new_local_candidates(&mut self) -> Vec<String> {
        self.transport.take_new_local_candidates()
    }

    pub fn state(&self) -> SessionState {
        if self.transport.is_connected() {
            SessionState::Connected
        } else {
            SessionState::Connecting
        }
    }

    /// DIAG: one-line connection snapshot (role/ICE state/selected pair/keyed).
    pub fn conn_debug(&self) -> String {
        self.transport.conn_debug()
    }

    pub fn is_connected(&self) -> bool {
        self.transport.is_connected()
    }

    /// Drain outbound datagrams (ICE/DTLS handshake + SRTP media, relay-routed as
    /// needed): `(dest, bytes)`. Media addressing + relay routing live in transport.
    pub fn poll_transmit(&mut self) -> Vec<(SocketAddr, Vec<u8>)> {
        self.transport.poll_transmit()
    }

    /// Feed one inbound datagram from `src`; demuxed to ICE/DTLS/media. Decoded
    /// PCM lands in `recv_audio`; reassembled video frames in `recv_video`.
    pub fn handle_datagram(&mut self, src: SocketAddr, data: &[u8]) -> Result<(), Error> {
        if let Demux::Media(srtp) = self.transport.handle_datagram(src, data)? {
            self.media_seen += 1;
            self.ensure_media()?;
            // RFC 5761 RTP/RTCP mux demux on byte 1: RTCP packet types occupy
            // 192–223 there, while an RTP M|PT byte for our PTs (101/102/108/118)
            // is <192 without the marker and >223 with it — no overlap.
            if srtp.len() >= 2 && (192..=223).contains(&srtp[1]) {
                self.handle_rtcp(&srtp);
                return Ok(());
            }
            if let Some(m) = &mut self.media {
                match m.recv(&srtp) {
                    // Empty = a non-Opus stream we skipped (video → the
                    // reassembler, telephone-event etc.) — not audio, not an error.
                    Ok(pcm) if pcm.is_empty() => {}
                    Ok(pcm) => { self.audio_in.push(pcm); self.decode_ok += 1; }
                    Err(_) => { self.decode_err += 1; }
                }
                while let Some(f) = m.take_video_frame() {
                    self.video_in.push(f);
                }
                // Inbound video loss → owe the peer a PLI (sent rate-limited from
                // `handle_timeout`; VP8 can't recover references without a keyframe).
                if m.take_video_loss() {
                    self.pli_wanted = true;
                }
                // ringrtc RTP-data control state from the peer (Phase 5).
                #[cfg(feature = "signal")]
                while let Some(b) = m.take_rtp_data() {
                    let Some(st) = crate::signal::decode_rtp_data(&b) else { continue };
                    if let Some(v) = st.video_enabled {
                        if self.peer_video_enabled != Some(v) {
                            self.peer_video_toggle = Some(v);
                        }
                        self.peer_video_enabled = Some(v);
                    }
                    if st.max_bitrate_bps.is_some() {
                        self.peer_max_bitrate = st.max_bitrate_bps;
                    }
                    if st.hangup {
                        self.peer_rtp_hangup = true;
                    }
                }
            }
        }
        Ok(())
    }

    /// One inbound SRTCP datagram: unprotect + scan the compound for what the
    /// video track acts on — PLI/FIR (peer wants a keyframe), REMB (peer's
    /// bandwidth estimate), SR (the peer's NTP⇄RTP sync anchor).
    fn handle_rtcp(&mut self, srtcp: &[u8]) {
        use rtc_rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
        use rtc_rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
        use rtc_rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate;
        use rtc_rtcp::sender_report::SenderReport;

        let Some(m) = &mut self.media else { return };
        let raw = match m.unprotect_rtcp(srtcp) {
            Ok(r) => r,
            Err(_) => {
                self.rtcp_err += 1;
                return;
            }
        };
        let mut buf = &raw[..];
        let Ok(pkts) = rtc_rtcp::packet::unmarshal(&mut buf) else {
            self.rtcp_err += 1;
            return;
        };
        for p in &pkts {
            let any = p.as_any();
            if any.downcast_ref::<PictureLossIndication>().is_some()
                || any.downcast_ref::<FullIntraRequest>().is_some()
            {
                // Addressed-SSRC check is deliberately loose: we send one video
                // stream, so any PLI/FIR from the peer can only mean it.
                self.keyframe_requested = true;
                self.pli_rx += 1;
            } else if let Some(remb) = any.downcast_ref::<ReceiverEstimatedMaximumBitrate>() {
                self.peer_remb_bps = Some(remb.bitrate as u32);
            } else if let Some(sr) = any.downcast_ref::<SenderReport>() {
                self.peer_sr = Some((sr.ssrc, sr.ntp_time, sr.rtp_time));
            }
        }
    }

    /// Build + queue the RTCP we owe: a rate-limited PLI when inbound video needs
    /// a keyframe, and a ~1 Hz sender report for our outbound video (the peer's
    /// A/V-sync anchor). Called from `handle_timeout`.
    fn pump_rtcp(&mut self, now: Instant) {
        use rtc_rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
        use rtc_rtcp::sender_report::SenderReport;
        use rtc_shared::marshal::Marshal;

        // Min spacing between PLIs — a keyframe answer takes a frame interval+RTT
        // anyway, and libwebrtc itself paces PLIs; this also caps the auto-PLI
        // storm a lossy link could otherwise trigger per dropped frame.
        const PLI_MIN_INTERVAL_MS: u128 = 300;
        const SR_INTERVAL_MS: u128 = 1000;

        let Some(m) = &mut self.media else { return };

        if self.pli_wanted {
            let due = self
                .last_pli_tx
                .is_none_or(|t| now.duration_since(t).as_millis() >= PLI_MIN_INTERVAL_MS);
            // The PLI targets the peer's video SSRC — unknown until its first
            // packet arrives; keep `pli_wanted` pending until then.
            if let (true, Some(media_ssrc)) = (due, m.video_rx_ssrc()) {
                let pli = PictureLossIndication {
                    sender_ssrc: m.video_ssrc().unwrap_or(0),
                    media_ssrc,
                };
                if let Ok(raw) = pli.marshal() {
                    if let Ok(dg) = m.protect_rtcp(&raw) {
                        self.transport.queue_media(dg);
                        self.pli_tx += 1;
                        self.last_pli_tx = Some(now);
                        self.pli_wanted = false;
                    }
                }
            }
        }

        // Sender report for the video stream once it has sent anything.
        if let (Some(ssrc), Some((pkts, octets, last_ts))) = (m.video_ssrc(), m.video_tx_stats()) {
            if pkts > 0 {
                let due = self
                    .last_sr_tx
                    .is_none_or(|t| now.duration_since(t).as_millis() >= SR_INTERVAL_MS);
                if due {
                    let sr = SenderReport {
                        ssrc,
                        ntp_time: ntp_now(),
                        rtp_time: last_ts,
                        packet_count: pkts,
                        octet_count: octets,
                        ..Default::default()
                    };
                    if let Ok(raw) = sr.marshal() {
                        if let Ok(dg) = m.protect_rtcp(&raw) {
                            self.transport.queue_media(dg);
                            self.last_sr_tx = Some(now);
                        }
                    }
                }
            }
        }

        // REMB ~1 Hz once we are RECEIVING video and have a budget: the
        // peer's sender-side BWE needs receiver feedback to ramp up (we send
        // no TWCC/RR). Shares the SR cadence via last_remb_tx.
        if let (Some(budget), Some(media_ssrc)) = (self.receive_budget_bps, m.video_rx_ssrc()) {
            use rtc_rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate;
            let due = self
                .last_remb_tx
                .is_none_or(|t| now.duration_since(t).as_millis() >= 1000);
            if due {
                let remb = ReceiverEstimatedMaximumBitrate {
                    sender_ssrc: m.video_ssrc().unwrap_or(0),
                    bitrate: budget as f32,
                    ssrcs: vec![media_ssrc],
                };
                if let Ok(raw) = remb.marshal() {
                    if let Ok(dg) = m.protect_rtcp(&raw) {
                        self.transport.queue_media(dg);
                        self.last_remb_tx = Some(now);
                    }
                }
            }
        }
    }

    /// Inbound-media diagnostics: `(seen, decode_ok, decode_err)`.
    pub fn media_diag(&self) -> (u64, u64, u64) {
        (self.media_seen, self.decode_ok, self.decode_err)
    }

    /// DIAG: inbound RTP stats `(seq_gaps, last_ts_step, last_payload_len)`; zeros
    /// until the media session exists.
    pub fn rtp_diag(&self) -> (u64, u32, usize) {
        self.media.as_ref().map(|m| m.rtp_diag()).unwrap_or((0, 0, 0))
    }

    /// DIAG: the peer's RTP `(payload_type, ssrc)`; zeros until media exists.
    pub fn rtp_peer_ids(&self) -> (u8, u32) {
        self.media.as_ref().map(|m| m.rtp_peer_ids()).unwrap_or((0, 0))
    }

    /// DIAG: last decode failure `(opus error string, failing TOC, stereo-bit count)`.
    pub fn dec_err_diag(&self) -> (&'static str, u8, u64) {
        self.media.as_ref().map(|m| m.dec_err_diag()).unwrap_or(("", 0, 0))
    }

    pub fn handle_timeout(&mut self, now: Instant) {
        self.transport.handle_timeout(now);
        let _ = self.ensure_media();
        self.pump_rtcp(now);
        // ringrtc resends the accumulated RTP-data state ~1 Hz; so do we once
        // there is anything to say (accepted engaged or a video state set).
        #[cfg(feature = "signal")]
        if (self.accepted_engaged || self.video_enabled_local.is_some())
            && self
                .last_rtp_data_tx
                .is_none_or(|t| now.duration_since(t).as_millis() >= 1000)
        {
            let _ = self.send_rtp_data_state();
        }
    }

    /// PCM frame length (samples) the codec expects per 20 ms tick.
    pub fn frame_len(&self) -> usize {
        SAMPLE_RATE as usize / 1000 * 20
    }

    /// Encode + protect one PCM frame → queued for the next `poll_transmit`.
    pub fn send_audio(&mut self, pcm: &[f32]) -> Result<(), Error> {
        self.ensure_media()?;
        let m = self.media.as_mut().ok_or(Error::NotConnected)?;
        let dg = m.send(pcm)?;
        self.transport.queue_media(dg);
        Ok(())
    }

    /// Next decoded PCM frame from the peer, if any.
    pub fn recv_audio(&mut self) -> Option<Vec<f32>> {
        if self.audio_in.is_empty() { None } else { Some(self.audio_in.remove(0)) }
    }

    // ── video track (task 93 Phase 3) ────────────────────────────────────────

    /// Packetize + protect one encoded VP8 frame (90 kHz `timestamp`, e.g. the
    /// host encoder's `next-frame` output) → queued for the next `poll_transmit`.
    pub fn send_video(&mut self, frame: &[u8], timestamp: u32) -> Result<(), Error> {
        self.ensure_media()?;
        let m = self.media.as_mut().ok_or(Error::NotConnected)?;
        for dg in m.send_video_frame(frame, timestamp)? {
            self.transport.queue_media(dg);
        }
        Ok(())
    }

    /// Next whole reassembled inbound video frame, if any — feed it to the host
    /// decoder (`wandr:video` `submit`).
    pub fn recv_video(&mut self) -> Option<VideoFrame> {
        if self.video_in.is_empty() { None } else { Some(self.video_in.remove(0)) }
    }

    /// True once if the peer asked for a keyframe (RTCP PLI/FIR) since the last
    /// call — answer with the encoder's `request-keyframe`.
    pub fn take_keyframe_request(&mut self) -> bool {
        std::mem::take(&mut self.keyframe_requested)
    }

    /// Ask the peer for a keyframe (our decoder lost sync, e.g. `queue-full`
    /// drops). Queues a rate-limited RTCP PLI on the next `handle_timeout`.
    pub fn request_keyframe(&mut self) {
        self.pli_wanted = true;
    }

    /// The peer's latest REMB bandwidth estimate (bps), if any — v1 congestion
    /// control: feed it to the encoder's `set-bitrate`.
    pub fn peer_remb_bps(&self) -> Option<u32> {
        self.peer_remb_bps
    }

    /// The peer's last RTCP sender report `(ssrc, ntp_time, rtp_time)` — the
    /// wall-clock ⇄ RTP-timestamp anchor for A/V sync.
    pub fn peer_sender_report(&self) -> Option<(u32, u64, u32)> {
        self.peer_sr
    }

    /// Video-plane counters: `((tx_frames, tx_pkts, rx_pkts, rx_frames,
    /// rx_broken), pli_tx, pli_rx, rtcp_err)`.
    pub fn video_diag(&self) -> (VideoDiag, u64, u64, u64) {
        let v = self.media.as_ref().map(|m| m.video_diag()).unwrap_or((0, 0, 0, 0, 0));
        (v, self.pli_tx, self.pli_rx, self.rtcp_err)
    }

    /// This session's role (offerer = caller, answerer = callee).
    pub fn role(&self) -> Role {
        self.role
    }

    /// Send ringrtc's `accepted` over the RTP-data control channel (PT 101 /
    /// SSRC 0xD), queued for the next `poll_transmit`. A 1:1 *caller* streams no
    /// media until it receives this from the *callee*, so the answerer must send
    /// it (repeatedly, ~1 Hz) once keyed. No-op-errs as `NotConnected` until the
    /// SRTP keys exist. `call_id` is the ringrtc call id (binds the message).
    /// (Phase 5: this now sends the ACCUMULATED state — accepted + our
    /// senderStatus — matching ringrtc's merge-and-resend semantics.)
    #[cfg(feature = "signal")]
    pub fn send_accepted(&mut self, call_id: u64) -> Result<(), Error> {
        self.rtp_call_id = Some(call_id);
        self.accepted_engaged = true;
        self.send_rtp_data_state()
    }

    /// Flip our camera state on the RTP-data channel (`senderStatus.
    /// video_enabled` — task 93 Phase 5). The peer's ringrtc surfaces it as
    /// remote-video on/off. Sent immediately when keyed and resent ~1 Hz with
    /// the accumulated state. `call_id` binds the message (callers never send
    /// `accepted`, so they pass it here).
    #[cfg(feature = "signal")]
    pub fn set_video_enabled(&mut self, call_id: u64, enabled: bool) {
        self.rtp_call_id = Some(call_id);
        self.video_enabled_local = Some(enabled);
        let _ = self.send_rtp_data_state(); // NotConnected → handle_timeout resends
    }

    /// Encode + queue the accumulated RTP-data `Message`.
    #[cfg(feature = "signal")]
    fn send_rtp_data_state(&mut self) -> Result<(), Error> {
        let Some(call_id) = self.rtp_call_id else { return Ok(()) };
        self.ensure_media()?;
        self.rtp_data_seqnum += 1;
        let payload = crate::signal::encode_rtp_data_state(
            call_id,
            self.rtp_data_seqnum,
            self.accepted_engaged,
            self.video_enabled_local,
            self.receive_budget_bps.map(u64::from),
        );
        let dg = self
            .media
            .as_mut()
            .ok_or(Error::NotConnected)?
            .send_rtp_data(crate::signal::RTP_DATA_PAYLOAD_TYPE, crate::signal::RTP_DATA_SSRC, &payload)?;
        self.transport.queue_media(dg);
        self.last_rtp_data_tx = Some(Instant::now());
        Ok(())
    }

    /// The peer toggled its camera (from `senderStatus.video_enabled`):
    /// `Some(true/false)` once per change, clear-on-read. The guest opens /
    /// closes the remote-video decoder surface on this.
    #[cfg(feature = "signal")]
    pub fn take_peer_video_toggle(&mut self) -> Option<bool> {
        self.peer_video_toggle.take()
    }

    /// The peer's current camera state, if it ever reported one.
    #[cfg(feature = "signal")]
    pub fn peer_video_enabled(&self) -> Option<bool> {
        self.peer_video_enabled
    }

    /// The send bitrate the peer requested (`receiverStatus.max_bitrate_bps`)
    /// — feed to the encoder's `set-bitrate` alongside REMB.
    #[cfg(feature = "signal")]
    pub fn peer_max_bitrate_bps(&self) -> Option<u64> {
        self.peer_max_bitrate
    }

    /// True once if the peer hung up over RTP data (it also hangs up via
    /// signaling; this is the faster in-band path). Clear-on-read.
    #[cfg(feature = "signal")]
    pub fn take_peer_rtp_hangup(&mut self) -> bool {
        std::mem::take(&mut self.peer_rtp_hangup)
    }

    /// Advertise our receive budget (bps) to the peer: RTCP REMB ~1 Hz (and
    /// the Signal path's rtp_data receiverStatus). Without ANY receiver
    /// feedback a libwebrtc sender parks at its minimum bitrate (~36 kbps
    /// observed live) — this is what un-starves inbound video quality.
    pub fn set_receive_bitrate(&mut self, bps: u32) {
        self.receive_budget_bps = Some(bps);
    }

    /// Set the outgoing CVO rotation (the camera's sensor orientation —
    /// degrees CW the receiver should apply for upright display). Carried in
    /// the `urn:3gpp:video-orientation` header extension, like libwebrtc.
    pub fn set_video_rotation(&mut self, degrees: u32) {
        self.video_rotation = Some(degrees);
        if let Some(m) = &mut self.media {
            m.set_video_rotation(degrees);
        }
    }

    fn ensure_media(&mut self) -> Result<(), Error> {
        if self.media.is_some() {
            return Ok(());
        }
        if let Some((send, recv)) = self.transport.take_keys() {
            let ssrc = if self.role == Role::Offerer { 0xA } else { 0xB };
            let profile = self.transport.srtp_profile();
            // Use the host AEAD backend if the guest injected one; else in-wasm AES.
            #[cfg(feature = "host-aead")]
            let mut media = match &self.aead_provider {
                Some(p) => MediaSession::new_with_aead(
                    SAMPLE_RATE, 1, self.audio_pt, ssrc, profile, &send, &recv, p.as_ref(),
                )?,
                None => MediaSession::new(SAMPLE_RATE, 1, self.audio_pt, ssrc, profile, &send, &recv)?,
            };
            #[cfg(not(feature = "host-aead"))]
            let mut media = MediaSession::new(SAMPLE_RATE, 1, self.audio_pt, ssrc, profile, &send, &recv)?;
            // Video SSRC scheme from ringrtc (signalapp/webrtc rffi
            // peer_connection.cc): BASE_SSRC = offerer 1000 / answerer 2000,
            // video = BASE+3 (audio is BASE+2, video RTX BASE+13 — unused here).
            // libwebrtc insists the two sides' video SSRCs don't overlap.
            let video_ssrc = if self.role == Role::Offerer { 1003 } else { 2003 };
            media.enable_video(VP8_PAYLOAD_TYPE, video_ssrc);
            if let Some(deg) = self.video_rotation {
                media.set_video_rotation(deg);
            }
            self.media = Some(media);
        }
        Ok(())
    }
}

/// Now as a 64-bit NTP timestamp (RFC 3550 SR format): seconds since 1900 in the
/// high half, fraction in the low. wasi's wall clock serves `SystemTime` in a guest.
fn ntp_now() -> u64 {
    /// Seconds between the NTP era (1900) and the Unix epoch (1970).
    const NTP_UNIX_OFFSET: u64 = 2_208_988_800;
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => {
            let secs = d.as_secs() + NTP_UNIX_OFFSET;
            let frac = (u64::from(d.subsec_nanos()) << 32) / 1_000_000_000;
            (secs << 32) | frac
        }
        Err(_) => 0,
    }
}

/// Build a standard SDP host-candidate string for an address.
fn candidate_string(addr: SocketAddr) -> String {
    format!("1 1 udp 2130706431 {} {} typ host", addr.ip(), addr.port())
}

/// A random ICE ufrag/pwd of `len` chars from the ICE-safe alphanumeric set.
#[cfg(feature = "signal")]
fn random_ice_cred(len: usize) -> String {
    use rand_core::RngCore;
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand_core::OsRng;
    (0..len)
        .map(|_| ALPHABET[(rng.next_u32() % ALPHABET.len() as u32) as usize] as char)
        .collect()
}

/// Parse the connection address from an SDP candidate string.
fn parse_candidate_addr(cand: &str) -> Option<SocketAddr> {
    // foundation component transport priority <ip> <port> typ host …
    let f: Vec<&str> = cand.split_whitespace().collect();
    if f.len() >= 6 {
        let ip: std::net::IpAddr = f[4].parse().ok()?;
        let port: u16 = f[5].parse().ok()?;
        return Some(SocketAddr::new(ip, port));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::UdpSocket;
    use std::time::Duration;

    /// Bind a peer to a real loopback UDP socket + a PeerSession on its address.
    fn peer(role: Role) -> (PeerSession, UdpSocket) {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.set_nonblocking(true).unwrap();
        let s = PeerSession::new(role, sock.local_addr().unwrap()).unwrap();
        (s, sock)
    }

    /// Drive two sessions over their real UDP sockets until both connect (or give
    /// up). Returns whether a fingerprint mismatch was raised.
    fn drive(
        a: &mut PeerSession, asock: &UdpSocket,
        b: &mut PeerSession, bsock: &UdpSocket,
    ) -> bool {
        let mut buf = [0u8; 2048];
        let mut mismatch = false;
        for _ in 0..1200 {
            let now = Instant::now();
            a.handle_timeout(now);
            b.handle_timeout(now);
            for (dest, data) in a.poll_transmit() { let _ = asock.send_to(&data, dest); }
            for (dest, data) in b.poll_transmit() { let _ = bsock.send_to(&data, dest); }
            while let Ok((n, src)) = asock.recv_from(&mut buf) {
                if matches!(a.handle_datagram(src, &buf[..n]), Err(Error::Dtls(_))) { mismatch = true; }
            }
            while let Ok((n, src)) = bsock.recv_from(&mut buf) {
                let _ = b.handle_datagram(src, &buf[..n]);
            }
            if (a.is_connected() && b.is_connected()) || mismatch { break; }
            std::thread::sleep(Duration::from_millis(3));
        }
        mismatch
    }

    /// Full call over REAL loopback UDP sockets: signaling → ICE → DTLS-SRTP →
    /// encrypted Opus media. This is repros/call-capstone over wasi:sockets-shaped
    /// transport (std::net::UdpSocket on host; wasi:sockets in the guest).
    #[test]
    fn two_peers_connect_over_real_udp() {
        let (mut a, asock) = peer(Role::Offerer);
        let (mut b, bsock) = peer(Role::Answerer);

        let offer = a.local_signaling().to_sdp();
        let answer = b.local_signaling().to_sdp();
        a.set_remote_signaling(&Signaling::from_sdp(&answer).unwrap()).unwrap();
        b.set_remote_signaling(&Signaling::from_sdp(&offer).unwrap()).unwrap();

        drive(&mut a, &asock, &mut b, &bsock);
        assert!(a.is_connected() && b.is_connected(), "failed to connect over real UDP");

        // A sends a tone → B decodes non-silent audio.
        let frame: Vec<f32> = (0..a.frame_len())
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SAMPLE_RATE as f32).sin() * 0.5)
            .collect();
        a.send_audio(&frame).unwrap();
        let mut buf = [0u8; 2048];
        for (dest, data) in a.poll_transmit() { let _ = asock.send_to(&data, dest); }
        std::thread::sleep(Duration::from_millis(20));
        while let Ok((n, src)) = bsock.recv_from(&mut buf) { b.handle_datagram(src, &buf[..n]).unwrap(); }
        let got = b.recv_audio().expect("B received an audio frame over UDP");
        let rms = (got.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / got.len() as f64).sqrt();
        assert!(rms > 0.0, "decoded audio present");
    }

    /// Pump every queued datagram from `from` into `to` over the real sockets.
    fn pump(
        from: &mut PeerSession, fsock: &UdpSocket,
        to: &mut PeerSession, tsock: &UdpSocket,
    ) {
        let mut buf = [0u8; 2048];
        for (dest, data) in from.poll_transmit() { let _ = fsock.send_to(&data, dest); }
        std::thread::sleep(Duration::from_millis(20));
        while let Ok((n, src)) = tsock.recv_from(&mut buf) {
            let _ = to.handle_datagram(src, &buf[..n]);
        }
    }

    /// Video track over real UDP: an encoded "VP8" frame larger than one MTU is
    /// fragmented (RFC 7741), SRTP-protected, reassembled whole on the far side
    /// with its timestamp and the keyframe bit recovered from the payload; a
    /// non-keyframe follows. The engine half of camera→encoder→RTP→decoder.
    #[test]
    fn video_frame_round_trip_over_udp() {
        let (mut a, asock) = peer(Role::Offerer);
        let (mut b, bsock) = peer(Role::Answerer);
        let offer = a.local_signaling().to_sdp();
        let answer = b.local_signaling().to_sdp();
        a.set_remote_signaling(&Signaling::from_sdp(&answer).unwrap()).unwrap();
        b.set_remote_signaling(&Signaling::from_sdp(&offer).unwrap()).unwrap();
        drive(&mut a, &asock, &mut b, &bsock);
        assert!(a.is_connected() && b.is_connected());

        // A fake keyframe (LSB of byte 0 = 0) spanning ~3 RTP fragments.
        let kf: Vec<u8> = (0..3000u32).map(|i| ((i * 7) as u8) & 0xFE).collect();
        a.send_video(&kf, 90_000).unwrap();
        pump(&mut a, &asock, &mut b, &bsock);
        let got = b.recv_video().expect("B reassembled the video frame");
        assert_eq!(got.data, kf, "frame bytes survive fragment+SRTP round trip");
        assert_eq!(got.timestamp, 90_000);
        assert!(got.keyframe, "keyframe bit recovered from the VP8 payload");

        // A small delta frame (LSB = 1).
        let delta = vec![0x01u8; 100];
        a.send_video(&delta, 93_000).unwrap();
        pump(&mut a, &asock, &mut b, &bsock);
        let got = b.recv_video().expect("delta frame arrives");
        assert!(!got.keyframe);
        assert!(b.recv_video().is_none(), "exactly two frames were sent");
        let ((tx_frames, tx_pkts, rx_pkts, rx_frames, rx_broken), ..) = b.video_diag();
        assert_eq!((tx_frames, tx_pkts), (0, 0), "B sent no video");
        assert_eq!(rx_frames, 2);
        assert!(rx_pkts >= 4, "multi-fragment keyframe + delta");
        assert_eq!(rx_broken, 0);
    }

    /// RTCP keyframe signaling both ways: the receiver's `request_keyframe`
    /// becomes a (SRTCP-protected, rate-limited) PLI on the wire, and the sender
    /// surfaces it via `take_keyframe_request` — the hook the guest answers with
    /// the HW encoder's `request-keyframe`. The sender's ~1 Hz video SR also
    /// lands as the peer's A/V-sync anchor.
    #[test]
    fn pli_and_sender_report_cross_the_wire() {
        let (mut a, asock) = peer(Role::Offerer);
        let (mut b, bsock) = peer(Role::Answerer);
        let offer = a.local_signaling().to_sdp();
        let answer = b.local_signaling().to_sdp();
        a.set_remote_signaling(&Signaling::from_sdp(&answer).unwrap()).unwrap();
        b.set_remote_signaling(&Signaling::from_sdp(&offer).unwrap()).unwrap();
        drive(&mut a, &asock, &mut b, &bsock);
        assert!(a.is_connected() && b.is_connected());

        // B must first learn A's video SSRC (the PLI's media_ssrc target).
        a.send_video(&[0x00; 64], 1000).unwrap();
        pump(&mut a, &asock, &mut b, &bsock);
        assert!(b.recv_video().is_some());

        assert!(!a.take_keyframe_request(), "no PLI yet");
        b.request_keyframe();
        b.handle_timeout(Instant::now()); // pump_rtcp queues the PLI
        pump(&mut b, &bsock, &mut a, &asock);
        assert!(a.take_keyframe_request(), "A surfaced the peer's PLI");
        assert!(!a.take_keyframe_request(), "request is clear-on-read");
        let (_, pli_tx_b, ..) = b.video_diag();
        assert_eq!(pli_tx_b, 1);

        // A has sent video → its next handle_timeout emits the video SR; B
        // stores the NTP⇄RTP anchor under A's video SSRC (offerer = 1003).
        a.handle_timeout(Instant::now());
        pump(&mut a, &asock, &mut b, &bsock);
        let (ssrc, ntp, rtp_ts) = b.peer_sender_report().expect("B received A's SR");
        assert_eq!(ssrc, 1003, "ringrtc scheme: offerer video SSRC = BASE 1000 + 3");
        assert!(ntp > 0);
        assert_eq!(rtp_ts, 1000, "SR anchors A's last sent RTP timestamp");
    }

    /// A lost fragment drops (only) the broken frame and auto-raises a PLI: the
    /// receiver never emits a torn frame to the decoder, the following intact
    /// frame still comes through, and the sender learns it must produce a
    /// keyframe — VP8's only recovery path without RTX.
    #[test]
    fn lost_fragment_drops_frame_and_auto_plis() {
        let (mut a, asock) = peer(Role::Offerer);
        let (mut b, bsock) = peer(Role::Answerer);
        let offer = a.local_signaling().to_sdp();
        let answer = b.local_signaling().to_sdp();
        a.set_remote_signaling(&Signaling::from_sdp(&answer).unwrap()).unwrap();
        b.set_remote_signaling(&Signaling::from_sdp(&offer).unwrap()).unwrap();
        drive(&mut a, &asock, &mut b, &bsock);
        assert!(a.is_connected() && b.is_connected());

        // A 3-fragment frame; drop the middle media datagram in transit.
        let torn: Vec<u8> = vec![0x00; 3000];
        a.send_video(&torn, 5000).unwrap();
        let mut media_seen = 0;
        for (dest, data) in a.poll_transmit() {
            // Video fragments are the only near-MTU datagrams in flight here.
            if data.len() > 500 {
                media_seen += 1;
                if media_seen == 2 {
                    continue; // the lost packet
                }
            }
            let _ = asock.send_to(&data, dest);
        }
        std::thread::sleep(Duration::from_millis(20));
        let mut buf = [0u8; 2048];
        while let Ok((n, src)) = bsock.recv_from(&mut buf) {
            let _ = b.handle_datagram(src, &buf[..n]);
        }
        assert!(b.recv_video().is_none(), "a torn frame must never reach the decoder");

        // The next intact frame still arrives.
        let intact = vec![0x01u8; 80];
        a.send_video(&intact, 8000).unwrap();
        pump(&mut a, &asock, &mut b, &bsock);
        let got = b.recv_video().expect("intact frame after the torn one");
        assert_eq!(got.data, intact);
        let ((_, _, _, rx_frames, rx_broken), ..) = b.video_diag();
        assert_eq!((rx_frames, rx_broken), (1, 1));

        // The loss auto-queued a PLI → A is told to produce a keyframe.
        b.handle_timeout(Instant::now());
        pump(&mut b, &bsock, &mut a, &asock);
        assert!(a.take_keyframe_request(), "auto-PLI from the torn frame reached A");
    }

    /// CVO: the sender's rotation (camera sensor orientation) rides the
    /// urn:3gpp:video-orientation header extension (id 4, ringrtc's fixed SDP)
    /// and comes out on the receiver's frame — no pixels touched.
    #[test]
    fn video_rotation_crosses_via_cvo_extension() {
        let (mut a, asock) = peer(Role::Offerer);
        let (mut b, bsock) = peer(Role::Answerer);
        let offer = a.local_signaling().to_sdp();
        let answer = b.local_signaling().to_sdp();
        a.set_remote_signaling(&Signaling::from_sdp(&answer).unwrap()).unwrap();
        b.set_remote_signaling(&Signaling::from_sdp(&offer).unwrap()).unwrap();
        drive(&mut a, &asock, &mut b, &bsock);
        assert!(a.is_connected() && b.is_connected());

        a.set_video_rotation(90); // e.g. a portrait phone's back camera
        a.send_video(&[0x00; 600], 3000).unwrap();
        pump(&mut a, &asock, &mut b, &bsock);
        let got = b.recv_video().expect("frame with CVO");
        assert_eq!(got.rotation, 90, "CVO rotation recovered on the receiver");

        // Sticky: a later sender omitting nothing changes — still 90.
        a.send_video(&[0x01; 100], 6000).unwrap();
        pump(&mut a, &asock, &mut b, &bsock);
        assert_eq!(b.recv_video().unwrap().rotation, 90);
    }

    /// Phase 5 in-call video signaling over the Signal (DH-keyed) path: the
    /// answerer's accepted+senderStatus reach the caller, video toggles fire
    /// exactly once per change (clear-on-read), and the accumulated state
    /// resends carry the latest value.
    #[cfg(feature = "signal")]
    #[test]
    fn video_enabled_toggle_crosses_rtp_data() {
        let caller_id: Vec<u8> = std::iter::once(0x05).chain(0..32).collect();
        let callee_id: Vec<u8> = std::iter::once(0x05).chain(32..64).collect();
        let bind = |role| {
            let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
            sock.set_nonblocking(true).unwrap();
            let s = PeerSession::new_signal(
                role, sock.local_addr().unwrap(), caller_id.clone(), callee_id.clone(), None,
            )
            .unwrap();
            (s, sock)
        };
        let (mut a, asock) = bind(Role::Offerer);
        let (mut b, bsock) = bind(Role::Answerer);
        let offer = a.local_signaling();
        let answer = b.local_signaling();
        a.set_remote_signaling(&answer).unwrap();
        b.set_remote_signaling(&offer).unwrap();
        drive(&mut a, &asock, &mut b, &bsock);
        assert!(a.is_connected() && b.is_connected());

        // Callee accepts + turns video ON (one accumulated message).
        b.send_accepted(0xC0FFEE).unwrap();
        b.set_video_enabled(0xC0FFEE, true);
        pump(&mut b, &bsock, &mut a, &asock);
        assert_eq!(a.take_peer_video_toggle(), Some(true), "caller saw video ON");
        assert_eq!(a.take_peer_video_toggle(), None, "toggle is clear-on-read");
        assert_eq!(a.peer_video_enabled(), Some(true));

        // Resends (same state) must NOT re-fire the toggle.
        b.handle_timeout(Instant::now() + Duration::from_secs(2));
        pump(&mut b, &bsock, &mut a, &asock);
        assert_eq!(a.take_peer_video_toggle(), None, "unchanged state, no event");

        // Video OFF → one new toggle.
        b.set_video_enabled(0xC0FFEE, false);
        pump(&mut b, &bsock, &mut a, &asock);
        assert_eq!(a.take_peer_video_toggle(), Some(false));
        assert_eq!(a.peer_video_enabled(), Some(false));
    }

    /// A tampered fingerprint (MITM) is rejected over real UDP — no connection.
    #[test]
    fn mismatched_fingerprint_rejected_over_udp() {
        let (mut a, asock) = peer(Role::Offerer);
        let (mut b, bsock) = peer(Role::Answerer);

        let offer = a.local_signaling().to_sdp();
        let mut b_sig = Signaling::from_sdp(&b.local_signaling().to_sdp()).unwrap();
        b_sig.fingerprint = format!("sha-256 {}", vec!["DE"; 32].join(":")); // bogus
        a.set_remote_signaling(&b_sig).unwrap();
        b.set_remote_signaling(&Signaling::from_sdp(&offer).unwrap()).unwrap();

        let mismatch = drive(&mut a, &asock, &mut b, &bsock);
        assert!(mismatch, "A must raise a fingerprint mismatch");
        assert!(!a.is_connected(), "A must not connect on fingerprint mismatch");
    }

    /// A remote TURN relay candidate's connection address (the relayed addr, fields
    /// 4/5) is extracted by `parse_candidate_addr`, ignoring the `typ relay raddr…` tail.
    #[test]
    fn relay_candidate_address_parses_to_relayed_addr() {
        let line = "1 1 udp 16777215 5.6.7.8 5000 typ relay raddr 1.2.3.4 6000";
        assert_eq!(parse_candidate_addr(line), Some("5.6.7.8:5000".parse().unwrap()));
    }

    /// Signal-mode call (ringrtc V4): NO DTLS — keys come from X25519-DH over the
    /// public keys exchanged in signaling. Two wandr peers connect over real UDP and
    /// exchange AEAD-AES-256-GCM-protected Opus. The analog of
    /// `two_peers_connect_over_real_udp`, proving the DH/GCM keying path end-to-end
    /// (the two-wandr half of Phase 2; real-Signal interop is Phase 3).
    #[cfg(feature = "signal")]
    #[test]
    fn two_peers_connect_signal_dh_over_real_udp() {
        // Both peers must agree on the same (caller, callee) identity keys — caller
        // = offerer. Stand-in 33-byte serialized identity pubkeys (0x05 ‖ 32).
        let caller_id: Vec<u8> = std::iter::once(0x05).chain(0..32).collect();
        let callee_id: Vec<u8> = std::iter::once(0x05).chain(32..64).collect();

        let bind = |role| {
            let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
            sock.set_nonblocking(true).unwrap();
            let s = PeerSession::new_signal(
                role, sock.local_addr().unwrap(), caller_id.clone(), callee_id.clone(), None,
            )
            .unwrap();
            (s, sock)
        };
        let (mut a, asock) = bind(Role::Offerer);
        let (mut b, bsock) = bind(Role::Answerer);

        // Signaling rides the `opaque` codec, not SDP — exchange Signaling directly
        // (it carries each peer's X25519 public_key + ICE creds/candidates).
        let offer = a.local_signaling();
        let answer = b.local_signaling();
        assert_eq!(offer.public_key.as_ref().map(|k| k.len()), Some(32), "offer carries X25519 key");
        assert!(offer.fingerprint.is_empty(), "Signal path has no DTLS fingerprint");
        a.set_remote_signaling(&answer).unwrap();
        b.set_remote_signaling(&offer).unwrap();

        drive(&mut a, &asock, &mut b, &bsock);
        assert!(a.is_connected() && b.is_connected(), "Signal-DH peers failed to connect");

        // A sends a tone → B decrypts (GCM) + decodes non-silent audio.
        let frame: Vec<f32> = (0..a.frame_len())
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SAMPLE_RATE as f32).sin() * 0.5)
            .collect();
        a.send_audio(&frame).unwrap();
        let mut buf = [0u8; 2048];
        for (dest, data) in a.poll_transmit() { let _ = asock.send_to(&data, dest); }
        std::thread::sleep(Duration::from_millis(20));
        while let Ok((n, src)) = bsock.recv_from(&mut buf) { b.handle_datagram(src, &buf[..n]).unwrap(); }
        let got = b.recv_audio().expect("B received a GCM-protected audio frame");
        let rms = (got.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / got.len() as f64).sqrt();
        assert!(rms > 0.0, "decoded audio present");
    }

    /// Mismatched identity keys (the HKDF `info`) → divergent SRTP keys → B cannot
    /// decrypt A's media. Guards that the identity binding actually feeds keying.
    #[cfg(feature = "signal")]
    #[test]
    fn signal_dh_identity_mismatch_breaks_media() {
        let bind = |role, caller: Vec<u8>, callee: Vec<u8>| {
            let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
            sock.set_nonblocking(true).unwrap();
            let s = PeerSession::new_signal(role, sock.local_addr().unwrap(), caller, callee, None).unwrap();
            (s, sock)
        };
        let (mut a, asock) = bind(Role::Offerer, vec![0x05; 33], vec![0xAA; 33]);
        // B uses a different callee identity → different HKDF info → different keys.
        let (mut b, bsock) = bind(Role::Answerer, vec![0x05; 33], vec![0xBB; 33]);

        a.set_remote_signaling(&b.local_signaling()).unwrap();
        b.set_remote_signaling(&a.local_signaling()).unwrap();
        drive(&mut a, &asock, &mut b, &bsock);
        // ICE/DH still "connect" (DH succeeds; only the derived keys differ).
        assert!(a.is_connected() && b.is_connected());

        let frame = vec![0.3f32; a.frame_len()];
        a.send_audio(&frame).unwrap();
        let mut buf = [0u8; 2048];
        for (dest, data) in a.poll_transmit() { let _ = asock.send_to(&data, dest); }
        std::thread::sleep(Duration::from_millis(20));
        // B's SRTP unprotect fails (wrong key) → the datagram errors, no audio.
        let mut got_audio = false;
        while let Ok((n, src)) = bsock.recv_from(&mut buf) {
            let _ = b.handle_datagram(src, &buf[..n]);
            if b.recv_audio().is_some() { got_audio = true; }
        }
        assert!(!got_audio, "mismatched identity keys must not yield decryptable media");
    }
}
