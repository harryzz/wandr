//! TWCC receiver feedback (task 93 Phase 5 quality fix).
//!
//! ringrtc negotiates transport-wide congestion control (header extension id
//! **1**, `TRANSPORT_CC1_EXT_ID` in its generated SDP), which puts a real
//! libwebrtc sender's bandwidth estimator in SEND-SIDE mode: it grows ONLY on
//! `TransportLayerCc` feedback from the receiver and otherwise backs off to its
//! minimum bitrate (~36 kbps observed live — the "peer video is blurry" bug).
//! This module is that receiver: it records the transport-wide sequence number
//! + arrival time of every inbound media packet and emits RFC-draft
//! `transport-cc` feedback ~10×/s, which the session SRTCP-protects and sends.
//!
//! Encoding kept deliberately simple and always-valid: two-bit status-vector
//! chunks (7 symbols each, padded with NotReceived past the count) covering the
//! contiguous sequence span of each batch; gaps are NotReceived, duplicates
//! keep the first arrival. Out-of-order arrivals WITHIN a batch are handled by
//! sorting; a packet older than an already-reported batch is dropped (the
//! sender treats unreported as lost — its next feedback recovers).

use std::time::Instant;

use rtc_rtcp::transport_feedbacks::transport_layer_cc::{
    PacketStatusChunk, RecvDelta, StatusVectorChunk, SymbolSizeTypeTcc, SymbolTypeTcc,
    TransportLayerCc,
};
use rtc_shared::marshal::Marshal;

/// The transport-wide sequence-number header extension id ringrtc fixes in its
/// SDP (`TRANSPORT_CC1_EXT_ID = 1`, rffi peer_connection.cc). 2-byte payload.
pub(crate) const TRANSPORT_CC_EXT_ID: u8 = 1;

/// Feedback cadence — libwebrtc sends transport-cc feedback every ~50–250 ms;
/// 100 ms keeps the sender's estimator responsive without RTCP spam.
const FEEDBACK_INTERVAL_MS: u128 = 100;

pub(crate) struct TwccTracker {
    /// `(transport-wide seq, arrival µs since `epoch`)` for the current batch.
    pkts: Vec<(u16, i64)>,
    epoch: Instant,
    fb_pkt_count: u8,
    last_fb: Option<Instant>,
    /// The SSRC most recently seen carrying the extension (used as media_ssrc).
    last_ssrc: u32,
}

impl TwccTracker {
    pub(crate) fn new() -> Self {
        Self {
            pkts: Vec::new(),
            epoch: Instant::now(),
            fb_pkt_count: 0,
            last_fb: None,
            last_ssrc: 0,
        }
    }

    /// Record one inbound media packet that carried the transport-wide seq ext.
    pub(crate) fn record(&mut self, seq: u16, ssrc: u32, now: Instant) {
        // Bound the batch (a stalled poll must not grow it unbounded).
        if self.pkts.len() < 2048 {
            self.pkts.push((seq, now.duration_since(self.epoch).as_micros() as i64));
        }
        self.last_ssrc = ssrc;
    }

    /// Build the next feedback packet if due: marshaled RTCP bytes, ready for
    /// SRTCP protection. `sender_ssrc` = any of our send SSRCs.
    pub(crate) fn poll(&mut self, now: Instant, sender_ssrc: u32) -> Option<Vec<u8>> {
        if self.pkts.is_empty() {
            return None;
        }
        if self
            .last_fb
            .is_some_and(|t| now.duration_since(t).as_millis() < FEEDBACK_INTERVAL_MS)
        {
            return None;
        }
        let mut batch = std::mem::take(&mut self.pkts);
        // Order by wrapping distance from the first arrival's seq, so reorder
        // within the batch is tolerated and wraparound is correct.
        let first = batch[0].0;
        batch.sort_by_key(|&(s, _)| (s.wrapping_sub(first)) as i16);
        batch.dedup_by_key(|&mut (s, _)| s);
        let min_off = (batch[0].0.wrapping_sub(first)) as i16 as i32;
        let max_off = (batch[batch.len() - 1].0.wrapping_sub(first)) as i16 as i32;
        let count = (max_off - min_off + 1) as usize;
        if count > 0x7FFF {
            return None; // pathological reorder span — skip this batch
        }
        let base_seq = first.wrapping_add(min_off as u16);

        // reference_time is in 64 ms units; deltas chain from it in 250 µs steps.
        let first_arrival = batch[0].1;
        let reference_time = ((first_arrival / 64_000) & 0x00FF_FFFF) as u32;
        let mut prev_us = reference_time as i64 * 64_000;

        let mut symbols = vec![SymbolTypeTcc::PacketNotReceived; count];
        let mut recv_deltas = Vec::with_capacity(batch.len());
        for &(s, t_us) in &batch {
            let idx = ((s.wrapping_sub(base_seq)) as i16 as i32) as usize;
            let delta_us = (t_us - prev_us).clamp(i16::MIN as i64 * 250, i16::MAX as i64 * 250);
            prev_us = t_us;
            let symbol = if (0..=255 * 250).contains(&delta_us) {
                SymbolTypeTcc::PacketReceivedSmallDelta
            } else {
                SymbolTypeTcc::PacketReceivedLargeDelta
            };
            symbols[idx] = symbol;
            recv_deltas.push(RecvDelta { type_tcc_packet: symbol, delta: delta_us });
        }

        // Two-bit status-vector chunks, 7 symbols each (NotReceived padding —
        // packet_status_count bounds what the parser reads).
        let packet_chunks: Vec<PacketStatusChunk> = symbols
            .chunks(7)
            .map(|c| {
                let mut list = c.to_vec();
                list.resize(7, SymbolTypeTcc::PacketNotReceived);
                PacketStatusChunk::StatusVectorChunk(StatusVectorChunk {
                    type_tcc: Default::default(),
                    symbol_size: SymbolSizeTypeTcc::TwoBit,
                    symbol_list: list,
                })
            })
            .collect();

        let fb = TransportLayerCc {
            sender_ssrc,
            media_ssrc: self.last_ssrc,
            base_sequence_number: base_seq,
            packet_status_count: count as u16,
            reference_time,
            fb_pkt_count: self.fb_pkt_count,
            packet_chunks,
            recv_deltas,
        };
        self.fb_pkt_count = self.fb_pkt_count.wrapping_add(1);
        self.last_fb = Some(now);
        fb.marshal().ok().map(|b| b.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtc_shared::marshal::Unmarshal;
    use std::time::Duration;

    /// A contiguous batch round-trips through marshal/unmarshal with the right
    /// base/count and one small delta per packet.
    #[test]
    fn contiguous_batch_builds_valid_feedback() {
        let mut t = TwccTracker::new();
        let t0 = t.epoch;
        for i in 0..10u16 {
            t.record(100 + i, 0x7d3, t0 + Duration::from_millis(20 * i as u64 + 5));
        }
        let bytes = t.poll(t0 + Duration::from_millis(300), 0xA).expect("feedback");
        let mut buf = &bytes[..];
        let fb = TransportLayerCc::unmarshal(&mut buf).expect("parse back");
        assert_eq!(fb.base_sequence_number, 100);
        assert_eq!(fb.packet_status_count, 10);
        assert_eq!(fb.media_ssrc, 0x7d3);
        assert_eq!(fb.recv_deltas.len(), 10);
        // Nothing due again until the next interval.
        assert!(t.poll(t0 + Duration::from_millis(310), 0xA).is_none());
    }

    /// A gap inside the batch shows as NotReceived (status count spans it) and
    /// reordered arrivals still produce a parseable packet.
    #[test]
    fn gap_and_reorder_are_tolerated() {
        let mut t = TwccTracker::new();
        let t0 = t.epoch;
        t.record(7, 1, t0 + Duration::from_millis(1));
        t.record(5, 1, t0 + Duration::from_millis(2)); // reordered
        t.record(9, 1, t0 + Duration::from_millis(3)); // gap: 6 and 8 missing
        let bytes = t.poll(t0 + Duration::from_millis(200), 0xA).expect("feedback");
        let mut buf = &bytes[..];
        let fb = TransportLayerCc::unmarshal(&mut buf).expect("parse back");
        assert_eq!(fb.base_sequence_number, 5);
        assert_eq!(fb.packet_status_count, 5); // 5,6,7,8,9
        assert_eq!(fb.recv_deltas.len(), 3);
    }
}
