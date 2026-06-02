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
    // DIAG (inbound): cumulative RTP-seq gaps (lost/missing packets), last
    // inter-packet timestamp delta (= Opus frame samples: 960=20ms, 2880=60ms),
    // and last payload size. Pins arrival rate / frame size / loss for the call.
    rx_prev_seq: Option<u16>,
    rx_prev_ts: Option<u32>,
    rx_seq_gaps: u64,
    rx_ts_step: u32,
    rx_payload_len: usize,
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
        Ok(Self {
            enc: OpusEncoder::new(sample_rate as _, channels as _, Application::Voip)
                .map_err(|_| Error::Codec("opus encoder"))?,
            dec: OpusDecoder::new(sample_rate as _, channels as _)
                .map_err(|_| Error::Codec("opus decoder"))?,
            tx: Context::new(&send.key, &send.salt, profile, None, None)
                .map_err(|_| Error::Srtp("send context"))?,
            rx: Context::new(&recv.key, &recv.salt, profile, None, None)
                .map_err(|_| Error::Srtp("recv context"))?,
            sample_rate,
            frame: sample_rate as usize / 1000 * 20, // 20 ms
            payload_type,
            ssrc,
            seq: 1,
            ts: 0,
            rx_prev_seq: None,
            rx_prev_ts: None,
            rx_seq_gaps: 0,
            rx_ts_step: 0,
            rx_payload_len: 0,
        })
    }

    /// DIAG: inbound RTP stats `(seq_gaps, last_ts_step, last_payload_len)`.
    pub fn rtp_diag(&self) -> (u64, u32, usize) {
        (self.rx_seq_gaps, self.rx_ts_step, self.rx_payload_len)
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

    /// Inbound SRTP datagram → RTP → decoded PCM frame.
    pub fn recv(&mut self, srtp: &[u8]) -> Result<Vec<f32>, Error> {
        let rtp = self.rx.decrypt_rtp(srtp).map_err(|_| Error::Srtp("decrypt"))?;
        let mut b = &rtp[..];
        let pkt = Packet::unmarshal(&mut b).map_err(|_| Error::Rtp("unmarshal"))?;
        // DIAG: track inbound RTP seq gaps + timestamp step + payload size.
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
        self.rx_payload_len = pkt.payload.len();
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
        let n = self.dec.decode(&pkt.payload, n_samples, &mut pcm).map_err(|_| Error::Codec("decode"))?;
        pcm.truncate(n);
        Ok(pcm)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}
