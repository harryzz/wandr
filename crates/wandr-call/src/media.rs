//! Media plane — the protocol-agnostic part of a call: PCM ⇄ encrypted RTP.
//!
//! Reused by every backend (WebRTC, and SIP/Jingle later) — only the keys, the
//! SRTP profile, and the wire differ. `MediaSession::send` takes a 20 ms PCM frame
//! and returns the SRTP datagram to put on the wire; `MediaSession::recv` takes an
//! inbound SRTP datagram and returns the decoded PCM. The SRTP keys + profile come
//! from the transport's key exchange (DTLS-SRTP for WebRTC-native; X25519-DH for
//! Signal — see `crate::signal` / `crate::transport`).
//!
//! Device-verified as repros/call-media-pipeline (38× real-time on a Pixel 2 XL).

use bytes::Bytes;
use opus_rs::{Application, OpusDecoder, OpusEncoder};
use rtc_rtp::header::Header;
use rtc_rtp::packet::Packet;
use rtc_shared::marshal::{Marshal, Unmarshal};
use rtc_srtp::context::Context;
use rtc_srtp::protection_profile::ProtectionProfile;

use crate::video::{VideoDiag, VideoFrame, VideoStream, VP8_RTX_PAYLOAD_TYPE};
use crate::Error;

/// Total decoded samples per channel in an Opus packet, from its TOC byte (the
/// standard `opus_packet_get_nb_samples` logic). We need the exact count to size
/// the decode buffer to the packet's real frame duration — opus-rs 0.1.22 panics
/// on a mismatched frame_size (see `MediaSession::recv`). Returns `None` on a
/// malformed packet.
fn opus_packet_samples(data: &[u8], sample_rate: u32) -> Option<usize> {
    let toc = *data.first()?;
    let config = (toc >> 3) as usize;
    // Samples/frame @ 48 kHz by codec mode + size bits (config & 3):
    //   SILK (0–11): 10/20/40/60 ms · hybrid (12–15): 10/20 ms · CELT (16–31): 2.5/5/10/20 ms
    let spf_48k = if config < 12 {
        [480, 960, 1920, 2880][config & 3]
    } else if config < 16 {
        [480, 960, 480, 960][config & 3]
    } else {
        [120, 240, 480, 960][config & 3]
    };
    let spf = spf_48k as usize * sample_rate as usize / 48_000;
    // Frame-count code (toc & 3): 0→1, 1/2→2 frames, 3→count in byte[1] low 6 bits.
    let frames = match toc & 0x3 {
        0 => 1,
        1 | 2 => 2,
        _ => (*data.get(1)? & 0x3F) as usize,
    };
    Some(spf * frames)
}

/// SRTP key + salt for one direction. Lengths depend on the profile:
/// AES128_CM_HMAC_SHA1_80 → 16 B key / 14 B salt; AEAD_AES_256_GCM → 32 B / 12 B.
pub struct SrtpKeys {
    pub key: Vec<u8>,
    pub salt: Vec<u8>,
}

/// One audio stream: Opus codec + RTP packetization + SRTP, both directions.
pub struct MediaSession {
    enc: OpusEncoder,
    dec: OpusDecoder,
    tx: Context, // protect outbound (our send keys)
    rx: Context, // unprotect inbound (peer's send keys)
    sample_rate: u32,
    frame: usize,
    payload_type: u8,
    ssrc: u32,
    seq: u16,
    ts: u32,
    // ringrtc RTP-data control channel (PT 101 / SSRC 0xD) shares our SRTP send
    // context but needs its own monotonic seq/timestamp; ringrtc drives both off
    // one counter, so we do too.
    data_ctr: u32,
    // The VP8 video track (task 93 Phase 3) — rides the same SRTP contexts on its
    // own SSRC. `None` until `enable_video` (the session enables it with the
    // role-derived SSRC as soon as media keys exist).
    video: Option<VideoStream>,
    // Inbound ringrtc RTP-data control payloads (PT 101) — decrypted protobuf
    // `rtp_data.Message` bytes, drained by the session (task 93 Phase 5: the
    // peer's video_enabled / max-bitrate / rtp hangup ride here).
    #[cfg(feature = "signal")]
    rtp_data_in: Vec<Vec<u8>>,
    // DIAG (inbound): cumulative RTP-seq gaps (lost/missing packets), last
    // inter-packet timestamp delta (= Opus frame samples: 960=20ms, 2880=60ms),
    // and last payload size. Pins arrival rate / frame size / loss for the call.
    rx_prev_seq: Option<u16>,
    rx_prev_ts: Option<u32>,
    rx_seq_gaps: u64,
    rx_ts_step: u32,
    rx_payload_len: usize,
    // DIAG: the PT + SSRC the PEER sends with — tells us what ringrtc expects us
    // to send on (its audio receiver demuxes by these; a mismatch = it drops us).
    rx_pt: u8,
    rx_ssrc: u32,
    // DIAG (decode failures): the opus decoder's own error string for the last
    // decode that failed, + that packet's TOC byte (mode/bandwidth/stereo bits) and
    // a count of failures whose TOC had the stereo bit set. Pins WHY packets are
    // rejected (channel mismatch vs SILK-decode-failed vs frame parsing).
    rx_dec_err_msg: &'static str,
    rx_dec_err_toc: u8,
    rx_dec_err_stereo: u64,
}

impl MediaSession {
    /// `send` keys protect our outbound media; `recv` keys unprotect the peer's.
    /// `profile` is the SRTP suite both keys are sized for (the key exchange picks
    /// it: AES128_CM_HMAC_SHA1_80 for DTLS-SRTP, AEAD_AES_256_GCM for Signal DH).
    pub fn new(
        sample_rate: u32,
        channels: u8,
        payload_type: u8,
        ssrc: u32,
        profile: ProtectionProfile,
        send: &SrtpKeys,
        recv: &SrtpKeys,
    ) -> Result<Self, Error> {
        let tx = Context::new(&send.key, &send.salt, profile, None, None)
            .map_err(|_| Error::Srtp("send context"))?;
        let rx = Context::new(&recv.key, &recv.salt, profile, None, None)
            .map_err(|_| Error::Srtp("recv context"))?;
        Self::from_contexts(sample_rate, channels, payload_type, ssrc, tx, rx)
    }

    /// Like [`MediaSession::new`], but the per-packet AES-GCM is delegated to a
    /// caller-supplied AEAD backend (feature `host-aead`). A wasm guest passes a
    /// provider that runs the GCM on the host's hardware AES via `wandr:crypto/aead`
    /// — ~3× faster for audio, ~8× for video vs the in-wasm software AES, with
    /// byte-identical SRTP framing (so it interops unchanged). `None` provider would
    /// just be `new`; this exists so the guest, not the library, owns the WIT.
    #[cfg(feature = "host-aead")]
    pub fn new_with_aead(
        sample_rate: u32,
        channels: u8,
        payload_type: u8,
        ssrc: u32,
        profile: ProtectionProfile,
        send: &SrtpKeys,
        recv: &SrtpKeys,
        provider: &dyn rtc_srtp::AeadProvider,
    ) -> Result<Self, Error> {
        let tx = Context::new_with_aead(&send.key, &send.salt, profile, None, None, provider)
            .map_err(|_| Error::Srtp("send context (aead)"))?;
        let rx = Context::new_with_aead(&recv.key, &recv.salt, profile, None, None, provider)
            .map_err(|_| Error::Srtp("recv context (aead)"))?;
        Self::from_contexts(sample_rate, channels, payload_type, ssrc, tx, rx)
    }

    /// Assemble a `MediaSession` from already-built SRTP contexts (the part common
    /// to [`MediaSession::new`] and [`MediaSession::new_with_aead`]).
    fn from_contexts(
        sample_rate: u32,
        channels: u8,
        payload_type: u8,
        ssrc: u32,
        tx: Context,
        rx: Context,
    ) -> Result<Self, Error> {
        Ok(Self {
            enc: OpusEncoder::new(sample_rate as _, channels as _, Application::Voip)
                .map_err(|_| Error::Codec("opus encoder"))?,
            dec: OpusDecoder::new(sample_rate as _, channels as _)
                .map_err(|_| Error::Codec("opus decoder"))?,
            tx,
            rx,
            sample_rate,
            frame: sample_rate as usize / 1000 * 20, // 20 ms
            payload_type,
            ssrc,
            seq: 1,
            ts: 0,
            data_ctr: 0,
            video: None,
            #[cfg(feature = "signal")]
            rtp_data_in: Vec::new(),
            rx_prev_seq: None,
            rx_prev_ts: None,
            rx_seq_gaps: 0,
            rx_ts_step: 0,
            rx_payload_len: 0,
            rx_pt: 0,
            rx_ssrc: 0,
            rx_dec_err_msg: "",
            rx_dec_err_toc: 0,
            rx_dec_err_stereo: 0,
        })
    }

    /// DIAG: the last decode failure — `(opus error string, failing TOC byte,
    /// count of failures with the stereo bit set)`. Empty string = no failure yet.
    pub fn dec_err_diag(&self) -> (&'static str, u8, u64) {
        (self.rx_dec_err_msg, self.rx_dec_err_toc, self.rx_dec_err_stereo)
    }

    /// DIAG: inbound RTP stats `(seq_gaps, last_ts_step, last_payload_len)`.
    pub fn rtp_diag(&self) -> (u64, u32, usize) {
        (self.rx_seq_gaps, self.rx_ts_step, self.rx_payload_len)
    }

    /// DIAG: the peer's RTP `(payload_type, ssrc)` — what ringrtc sends with, and
    /// (by symmetry) what its audio receiver demuxes for. We send PT 102 / ssrc
    /// 0xA|0xB; if the peer uses something else, it likely drops our stream.
    pub fn rtp_peer_ids(&self) -> (u8, u32) {
        (self.rx_pt, self.rx_ssrc)
    }

    /// Frames per 20 ms tick at this sample rate (e.g. 960 @ 48 kHz).
    pub fn frame_len(&self) -> usize {
        self.frame
    }

    /// Encode one PCM frame → RTP → SRTP datagram to send on the wire.
    pub fn send(&mut self, pcm: &[f32]) -> Result<Vec<u8>, Error> {
        let mut buf = [0u8; 4000];
        let n = self.enc.encode(pcm, self.frame, &mut buf).map_err(|_| Error::Codec("encode"))?;
        let pkt = Packet {
            header: Header {
                version: 2,
                payload_type: self.payload_type,
                sequence_number: self.seq,
                timestamp: self.ts,
                ssrc: self.ssrc,
                ..Default::default()
            },
            payload: Bytes::copy_from_slice(&buf[..n]),
        };
        let rtp = pkt.marshal().map_err(|_| Error::Rtp("marshal"))?;
        let srtp = self.tx.encrypt_rtp(&rtp).map_err(|_| Error::Srtp("encrypt"))?;
        self.seq = self.seq.wrapping_add(1);
        self.ts = self.ts.wrapping_add(self.frame as u32);
        Ok(srtp.to_vec())
    }

    /// Packetize + SRTP-protect an opaque control payload on a side channel
    /// (`pt`/`ssrc`), e.g. ringrtc's RTP-data channel (PT 101 / SSRC 0xD). Uses a
    /// dedicated monotonic counter for both seq and timestamp (ringrtc does the
    /// same) and the SAME send SRTP context — an SRTP context keys each SSRC's
    /// keystream independently, so multiplexing a second SSRC is correct.
    pub fn send_rtp_data(&mut self, pt: u8, ssrc: u32, payload: &[u8]) -> Result<Vec<u8>, Error> {
        self.data_ctr = self.data_ctr.wrapping_add(1);
        let pkt = Packet {
            header: Header {
                version: 2,
                payload_type: pt,
                sequence_number: self.data_ctr as u16,
                timestamp: self.data_ctr,
                ssrc,
                ..Default::default()
            },
            payload: Bytes::copy_from_slice(payload),
        };
        let rtp = pkt.marshal().map_err(|_| Error::Rtp("marshal"))?;
        let srtp = self.tx.encrypt_rtp(&rtp).map_err(|_| Error::Srtp("encrypt"))?;
        Ok(srtp.to_vec())
    }

    /// Inbound SRTP datagram → RTP → decoded PCM frame. Returns an EMPTY vec for a
    /// packet that isn't our Opus audio stream (a benign skip, not an error): a
    /// real ringrtc peer multiplexes several streams onto the one port — the Opus
    /// audio (e.g. PT 102 / ssrc 0x7d2) interleaved with a telephone-event / other
    /// stream (PT 101) — and feeding those non-Opus payloads to the decoder
    /// produced ~16 % spurious decode errors and made the seq/ts diag meaningless
    /// (gaps in the millions from interleaved SSRCs). Filter to our audio PT.
    pub fn recv(&mut self, srtp: &[u8]) -> Result<Vec<f32>, Error> {
        let rtp = self.rx.decrypt_rtp(srtp).map_err(|_| Error::Srtp("decrypt"))?;
        let mut b = &rtp[..];
        let pkt = Packet::unmarshal(&mut b).map_err(|_| Error::Rtp("unmarshal"))?;
        // DIAG (all streams): the last PT/SSRC/payload size seen on the wire.
        self.rx_pt = pkt.header.payload_type;
        self.rx_ssrc = pkt.header.ssrc;
        self.rx_payload_len = pkt.payload.len();
        // The video track (VP8 + its RTX) demuxes by PT into the reassembler;
        // whole frames come out via `take_video_frame` (empty PCM here = not audio).
        if let Some(v) = &mut self.video {
            if pkt.header.payload_type == v.pt()
                || pkt.header.payload_type == VP8_RTX_PAYLOAD_TYPE
            {
                v.handle_rtp(&pkt);
                return Ok(Vec::new());
            }
        }
        // ringrtc's RTP-data control channel (PT 101 / SSRC 0xD): queue the
        // protobuf payload for the session to decode (video_enabled etc.).
        #[cfg(feature = "signal")]
        if pkt.header.payload_type == crate::signal::RTP_DATA_PAYLOAD_TYPE
            && pkt.header.ssrc == crate::signal::RTP_DATA_SSRC
        {
            if self.rtp_data_in.len() < 16 {
                self.rtp_data_in.push(pkt.payload.to_vec());
            }
            return Ok(Vec::new());
        }
        // Only the Opus audio stream is ours to decode; skip everything else so the
        // decoder + the seq/ts/gap stats see a single, coherent stream.
        if pkt.header.payload_type != self.payload_type {
            return Ok(Vec::new());
        }
        // DIAG (audio stream only): seq gaps + timestamp step — now meaningful.
        let seq = pkt.header.sequence_number;
        let ts = pkt.header.timestamp;
        if let Some(prev) = self.rx_prev_seq {
            // Packets missing between the last seq and this one (0 if contiguous).
            self.rx_seq_gaps += seq.wrapping_sub(prev.wrapping_add(1)) as u64;
        }
        if let Some(prev_ts) = self.rx_prev_ts {
            self.rx_ts_step = ts.wrapping_sub(prev_ts);
        }
        self.rx_prev_seq = Some(seq);
        self.rx_prev_ts = Some(ts);
        // Decode at the packet's EXACT sample count, read from its Opus TOC. The peer
        // packetizes 60 ms frames (2880 samples), not our 20 ms — decoding into a
        // 960-sample buffer dropped 2/3 of every packet (garbled). But opus-rs 0.1.22
        // has a CELT bug (quant_bands E_PROB_MODEL[lm] out of bounds, panics → SIGILL)
        // whenever frame_size is a *mismatched* multiple of the real frame, so a fixed
        // oversized buffer crashes. Sizing to the packet's own duration matches the
        // decoder's expectation and avoids the bug.
        let n_samples = opus_packet_samples(&pkt.payload, self.sample_rate)
            .unwrap_or(self.frame)
            .max(self.frame);
        let mut pcm = vec![0f32; n_samples];
        let n = match self.dec.decode(&pkt.payload, n_samples, &mut pcm) {
            Ok(n) => n,
            Err(e) => {
                // Preserve the decoder's own reason + the packet's TOC so calldbg can
                // show WHY (channel mismatch / SILK-decode-failed / frame parse).
                self.rx_dec_err_msg = e;
                self.rx_dec_err_toc = pkt.payload.first().copied().unwrap_or(0);
                if self.rx_dec_err_toc & 0x04 != 0 { self.rx_dec_err_stereo += 1; }
                return Err(Error::Codec("decode"));
            }
        };
        pcm.truncate(n);
        Ok(pcm)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    // ── video track (task 93 Phase 3) ────────────────────────────────────────

    /// Enable the VP8 video track on this session: `pt` is the VP8 payload type
    /// ([`crate::video::VP8_PAYLOAD_TYPE`] for ringrtc/libwebrtc peers), `ssrc`
    /// our video send SSRC. Idempotent.
    pub fn enable_video(&mut self, pt: u8, ssrc: u32) {
        if self.video.is_none() {
            self.video = Some(VideoStream::new(pt, ssrc));
        }
    }

    /// One encoded VP8 frame (90 kHz `timestamp`) → the SRTP datagrams to send
    /// (one per RTP fragment, marker on the last).
    pub fn send_video_frame(&mut self, data: &[u8], timestamp: u32) -> Result<Vec<Vec<u8>>, Error> {
        let v = self.video.as_mut().ok_or(Error::Rtp("video not enabled"))?;
        v.payload_frame(&mut self.tx, data, timestamp)
    }

    /// Next whole reassembled inbound video frame, if any.
    pub fn take_video_frame(&mut self) -> Option<VideoFrame> {
        self.video.as_mut().and_then(|v| v.take_frame())
    }

    /// True once if inbound video loss/drops occurred since the last call — the
    /// session's PLI trigger.
    pub fn take_video_loss(&mut self) -> bool {
        self.video.as_mut().is_some_and(|v| v.take_loss())
    }

    /// Our video send SSRC (RTCP SR sender), if video is enabled.
    pub fn video_ssrc(&self) -> Option<u32> {
        self.video.as_ref().map(|v| v.ssrc())
    }

    /// The peer's video SSRC once learned from its first packet (PLI target).
    pub fn video_rx_ssrc(&self) -> Option<u32> {
        self.video.as_ref().and_then(|v| v.rx_ssrc())
    }

    /// Video send totals for the RTCP SR: `(packets, octets, last_rtp_ts)`.
    pub fn video_tx_stats(&self) -> Option<(u32, u32, u32)> {
        self.video.as_ref().map(|v| v.tx_stats())
    }

    /// Video-plane counters; zeros until video is enabled.
    pub fn video_diag(&self) -> VideoDiag {
        self.video.as_ref().map(|v| v.diag()).unwrap_or((0, 0, 0, 0, 0))
    }

    /// Set the outgoing CVO rotation on the video track (camera sensor
    /// orientation; degrees CW the receiver should apply).
    pub fn set_video_rotation(&mut self, degrees: u32) {
        if let Some(v) = &mut self.video {
            v.set_rotation(degrees);
        }
    }

    /// Next inbound RTP-data control payload (PT 101 protobuf), if any.
    #[cfg(feature = "signal")]
    pub fn take_rtp_data(&mut self) -> Option<Vec<u8>> {
        if self.rtp_data_in.is_empty() {
            None
        } else {
            Some(self.rtp_data_in.remove(0))
        }
    }

    // ── RTCP (SRTCP rides the same contexts; RFC 5761 mux) ──────────────────

    /// Protect an RTCP compound payload → SRTCP datagram (our send context).
    pub fn protect_rtcp(&mut self, raw: &[u8]) -> Result<Vec<u8>, Error> {
        Ok(self.tx.encrypt_rtcp(raw).map_err(|_| Error::Srtp("encrypt rtcp"))?.to_vec())
    }

    /// Unprotect an inbound SRTCP datagram → the RTCP compound payload.
    pub fn unprotect_rtcp(&mut self, data: &[u8]) -> Result<Vec<u8>, Error> {
        Ok(self.rx.decrypt_rtcp(data).map_err(|_| Error::Srtp("decrypt rtcp"))?.to_vec())
    }
}
