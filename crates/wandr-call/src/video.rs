//! Video plane — the VP8 RTP video track (task 93 Phase 3).
//!
//! Rides the SAME per-direction SRTP contexts as the audio in [`crate::media`]
//! (an SRTP context keys each SSRC's keystream independently, so multiplexing the
//! video SSRC through the audio session's contexts is correct — the same property
//! `send_rtp_data` already relies on). wandr-call stays codec- and WIT-agnostic
//! here too: the engine moves **encoded VP8 frames** (`Vec<u8>` + 90 kHz RTP
//! timestamp); the consuming guest wires them to the host's HW codec
//! (`wandr:video` encoder `next-frame` / decoder `submit`) — pixels never enter
//! the engine.
//!
//! Outbound: one encoded frame → RFC 7741 VP8 payload descriptors (fragmented at
//! the RTP MTU, PictureID enabled) → RTP (marker on the last fragment) → SRTP.
//! Inbound: per-packet depacketize → in-order frame reassembly keyed by the S bit
//! + timestamp → whole frames out; a hole inside a frame (or a missing start)
//! drops the frame and raises the loss flag, which the session turns into a
//! rate-limited RTCP PLI (VP8's reference chain is broken until a keyframe).

use bytes::Bytes;
use rtc_rtp::codec::vp8::{Vp8Packet, Vp8Payloader};
use rtc_rtp::header::Header;
use rtc_rtp::packet::Packet;
use rtc_rtp::packetizer::{Depacketizer, Payloader};
use rtc_shared::marshal::Marshal;
use rtc_srtp::context::Context;

use crate::Error;

/// VP8 dynamic payload type — **108**, the value ringrtc fixes in the local SDP
/// it generates on BOTH ends of a 1:1 call (signalapp/webrtc
/// `ringrtc/rffi/src/peer_connection.cc`: `VP8_PT = 108`, beside the proven
/// `OPUS_PT = 102`). Like Opus, the PT must be in the peer's receive map for
/// libwebrtc to accept our (unsignaled-SSRC) stream; the wrong PT is dropped
/// silently — the exact failure mode the audio PT hunt already paid for.
pub const VP8_PAYLOAD_TYPE: u8 = 108;
/// ringrtc's RTX (retransmission, RFC 4588) payload type for VP8 (`VP8_RTX_PT`).
/// We don't implement RTX; inbound RTX packets are counted + ignored.
pub const VP8_RTX_PAYLOAD_TYPE: u8 = 118;

/// Outbound RTP payload budget per packet: the standard WebRTC datagram budget
/// (1200 — what libwebrtc paces to, safely under every real path MTU) minus the
/// 12-byte RTP header and the worst-case SRTP auth trailer (16-byte GCM tag).
/// The VP8 payload descriptor is inside this budget (the payloader's `mtu`).
const RTP_PAYLOAD_BUDGET: usize = 1200 - 12 - 16;

/// One whole encoded VP8 frame crossing the engine boundary (guest ⇄ engine).
/// `timestamp` is the 90 kHz RTP timestamp (the same clock `wandr:video` uses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    pub data: Vec<u8>,
    pub timestamp: u32,
    pub keyframe: bool,
}

/// Aggregate video-plane counters: `(tx_frames, tx_packets, rx_packets,
/// rx_frames, rx_broken_frames)`.
pub type VideoDiag = (u64, u32, u64, u64, u64);

/// The VP8 RTP stream state, both directions. Owned by `MediaSession` (which
/// owns the SRTP contexts it borrows per call).
pub(crate) struct VideoStream {
    pt: u8,
    ssrc: u32,
    payloader: Vp8Payloader,
    seq: u16,
    // tx counters (RTCP SR + diag)
    tx_frames: u64,
    tx_packets: u32,
    tx_octets: u32,
    last_tx_ts: u32,
    // rx reassembly
    rx_ssrc: Option<u32>,
    prev_seq: Option<u16>,
    cur: Vec<u8>,
    cur_ts: u32,
    assembling: bool,
    broken: bool,
    /// Set when inbound frames were lost/dropped — the session drains this into
    /// a rate-limited PLI.
    loss: bool,
    frames_out: Vec<VideoFrame>,
    // rx counters
    rx_packets: u64,
    rx_frames: u64,
    rx_broken_frames: u64,
}

impl VideoStream {
    pub(crate) fn new(pt: u8, ssrc: u32) -> Self {
        // PictureID gives libwebrtc's jitter buffer explicit frame identity
        // (it tolerates absence, but real ringrtc senders include it).
        let mut payloader = Vp8Payloader::default();
        payloader.enable_picture_id = true;
        Self {
            pt,
            ssrc,
            payloader,
            seq: 1,
            tx_frames: 0,
            tx_packets: 0,
            tx_octets: 0,
            last_tx_ts: 0,
            rx_ssrc: None,
            prev_seq: None,
            cur: Vec::new(),
            cur_ts: 0,
            assembling: false,
            broken: false,
            loss: false,
            frames_out: Vec::new(),
            rx_packets: 0,
            rx_frames: 0,
            rx_broken_frames: 0,
        }
    }

    pub(crate) fn pt(&self) -> u8 {
        self.pt
    }

    /// Our send SSRC (the RTCP SR sender).
    pub(crate) fn ssrc(&self) -> u32 {
        self.ssrc
    }

    /// The peer's video SSRC, once learned from its first packet — the PLI's
    /// `media_ssrc` target.
    pub(crate) fn rx_ssrc(&self) -> Option<u32> {
        self.rx_ssrc
    }

    /// Send-side totals for the RTCP sender report: `(packets, octets, last_ts)`.
    pub(crate) fn tx_stats(&self) -> (u32, u32, u32) {
        (self.tx_packets, self.tx_octets, self.last_tx_ts)
    }

    pub(crate) fn diag(&self) -> VideoDiag {
        (self.tx_frames, self.tx_packets, self.rx_packets, self.rx_frames, self.rx_broken_frames)
    }

    /// One encoded frame → fragmented VP8 RTP → SRTP datagrams (marker on the
    /// last fragment). `tx` is the media session's send SRTP context.
    pub(crate) fn payload_frame(
        &mut self,
        tx: &mut Context,
        data: &[u8],
        timestamp: u32,
    ) -> Result<Vec<Vec<u8>>, Error> {
        let fragments = self
            .payloader
            .payload(RTP_PAYLOAD_BUDGET, &Bytes::copy_from_slice(data))
            .map_err(|_| Error::Rtp("vp8 payload"))?;
        let last = fragments.len().saturating_sub(1);
        let mut out = Vec::with_capacity(fragments.len());
        for (i, frag) in fragments.into_iter().enumerate() {
            let payload_len = frag.len() as u32;
            let pkt = Packet {
                header: Header {
                    version: 2,
                    payload_type: self.pt,
                    sequence_number: self.seq,
                    timestamp,
                    ssrc: self.ssrc,
                    marker: i == last,
                    ..Default::default()
                },
                payload: frag,
            };
            let rtp = pkt.marshal().map_err(|_| Error::Rtp("marshal"))?;
            let srtp = tx.encrypt_rtp(&rtp).map_err(|_| Error::Srtp("encrypt"))?;
            self.seq = self.seq.wrapping_add(1);
            self.tx_packets = self.tx_packets.wrapping_add(1);
            self.tx_octets = self.tx_octets.wrapping_add(payload_len);
            out.push(srtp.to_vec());
        }
        self.tx_frames += 1;
        self.last_tx_ts = timestamp;
        Ok(out)
    }

    /// Feed one decrypted+parsed inbound RTP packet of our video PT (or RTX).
    /// Whole reassembled frames land in `take_frame`; loss raises `take_loss`.
    pub(crate) fn handle_rtp(&mut self, pkt: &Packet) {
        if pkt.header.payload_type == VP8_RTX_PAYLOAD_TYPE {
            return; // RTX not implemented — recovery is PLI → keyframe
        }
        if pkt.header.payload_type != self.pt {
            return;
        }
        // Lock onto the first video SSRC seen (no simulcast on a 1:1 call).
        match self.rx_ssrc {
            None => self.rx_ssrc = Some(pkt.header.ssrc),
            Some(s) if s != pkt.header.ssrc => return,
            _ => {}
        }
        self.rx_packets += 1;
        let seq = pkt.header.sequence_number;
        let contiguous = self.prev_seq.is_none_or(|p| seq == p.wrapping_add(1));
        self.prev_seq = Some(seq);

        let mut vp8 = Vp8Packet::default();
        let payload = match vp8.depacketize(&pkt.payload) {
            Ok(p) => p,
            Err(_) => {
                self.drop_current(true);
                return;
            }
        };
        // RFC 7741: S=1 + PID=0 marks the first packet of a frame.
        let start = vp8.s == 1 && vp8.pid == 0;
        if start {
            if self.assembling {
                // A new frame began while one was mid-assembly → the old one
                // lost its tail.
                self.drop_current(true);
            }
            self.cur.clear();
            self.cur_ts = pkt.header.timestamp;
            self.assembling = true;
            self.broken = false;
            // A gap right before a frame start = whole frame(s) lost between
            // frames; this frame itself is intact, but VP8 references are not.
            if !contiguous {
                self.loss = true;
            }
        } else if !self.assembling {
            // Continuation without a start (its S packet was lost) — unusable.
            self.loss = true;
            return;
        } else if !contiguous || pkt.header.timestamp != self.cur_ts {
            // Hole inside the frame, or fragments of two frames interleaved.
            self.broken = true;
        }
        self.cur.extend_from_slice(&payload);
        if pkt.header.marker {
            if self.broken {
                self.drop_current(true);
            } else {
                // VP8 uncompressed-data-chunk P bit (LSB of byte 0): 0 = keyframe.
                let keyframe = self.cur.first().is_some_and(|b| b & 0x01 == 0);
                self.frames_out.push(VideoFrame {
                    data: std::mem::take(&mut self.cur),
                    timestamp: self.cur_ts,
                    keyframe,
                });
                self.rx_frames += 1;
                self.assembling = false;
            }
        }
    }

    pub(crate) fn take_frame(&mut self) -> Option<VideoFrame> {
        if self.frames_out.is_empty() {
            None
        } else {
            Some(self.frames_out.remove(0))
        }
    }

    /// True once if inbound loss/drops occurred since the last call (PLI trigger).
    pub(crate) fn take_loss(&mut self) -> bool {
        std::mem::take(&mut self.loss)
    }

    fn drop_current(&mut self, count: bool) {
        if count && (self.assembling || !self.cur.is_empty()) {
            self.rx_broken_frames += 1;
        }
        self.cur.clear();
        self.assembling = false;
        self.broken = false;
        self.loss = true;
    }
}
