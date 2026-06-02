//! Media plane — the protocol-agnostic part of a call: PCM ⇄ encrypted RTP.
//!
//! Reused by every backend (WebRTC, and SIP/Jingle later) — only the keys and
//! the wire differ. `MediaSession::send` takes a 20 ms PCM frame and returns the
//! SRTP datagram to put on the wire; `MediaSession::recv` takes an inbound SRTP
//! datagram and returns the decoded PCM. The SRTP keys come from the transport's
//! key exchange (DTLS-SRTP for WebRTC).
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

/// SRTP_AES128_CM_HMAC_SHA1_80 key (16 B) + salt (14 B) for one direction.
pub struct SrtpKeys {
    pub key: [u8; 16],
    pub salt: [u8; 14],
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
}

impl MediaSession {
    /// `send` keys protect our outbound media; `recv` keys unprotect the peer's.
    pub fn new(
        sample_rate: u32,
        channels: u8,
        payload_type: u8,
        ssrc: u32,
        send: &SrtpKeys,
        recv: &SrtpKeys,
    ) -> Result<Self, Error> {
        let profile = ProtectionProfile::Aes128CmHmacSha1_80;
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
        })
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
        let mut pcm = vec![0f32; self.frame];
        let n = self.dec.decode(&pkt.payload, self.frame, &mut pcm).map_err(|_| Error::Codec("decode"))?;
        pcm.truncate(n);
        Ok(pcm)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}
