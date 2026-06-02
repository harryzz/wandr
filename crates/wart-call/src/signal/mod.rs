//! Signal 1:1 call protocol — the `opaque` ⇄ [`Signaling`] codec.
//!
//! Signal carries call setup in an `opaque` bytes blob inside the E2E-encrypted
//! `CallMessage.{Offer,Answer,IceUpdate}` (see
//! `external/libsignal-service-rs/protobuf/SignalService.proto`). That blob is a
//! protobuf of ringrtc's *connection parameters* — its own wire format, not SDP.
//! This module decodes/encodes that blob to/from [`Signaling`], so the rest of
//! [`crate`] (transport/media) is reused unchanged. The consuming Signal engine
//! assembles the surrounding `CallMessage` (it already has libsignal's generated
//! types); this codec deals only in the `opaque` bytes.
//!
//! The reference contract is `crates/wart-call/proto/signal_signaling.proto`. We
//! hand-derive [`prost::Message`] structs from it ([`proto`]) rather than running
//! protoc — the wire bytes are identical and the build needs no extra toolchain.
//!
//! # The big caveat for Phase 2 (deferred — NOT implemented here)
//!
//! Signal's **V4** 1:1 protocol does **not use DTLS-SRTP**. There is no DTLS
//! fingerprint and no `a=setup` role on the wire — [`ConnectionParametersV4`]
//! instead carries an **X25519 `public_key`**. Both peers derive raw SRTP keys via
//! an X25519 Diffie-Hellman exchange:
//!
//! ```text
//! shared = X25519(local_secret, remote_public_key)
//! okm    = HKDF-SHA256(ikm=shared, salt=[0u8;32],
//!            info="Signal_Calling_20200807_SignallingDH_SRTPKey_KDF"
//!                 || caller_identity_key || callee_identity_key)
//! layout = offer_key(32) || offer_salt(12) || answer_key(32) || answer_salt(12)
//! suite  = AEAD AES-256-GCM   (NOT the AES128_CM_HMAC_SHA1_80 wart-call hardcodes)
//! ```
//!
//! So Phase 2 must, in `transport.rs`/`media.rs`: skip the DTLS handshake on the
//! Signal path, do X25519 keygen + DH (the caller/callee ACI identity keys come
//! from the Signal engine's `IdentityKeyPair`, `apps/user/war.signal/engine/src/
//! store.rs`), and widen `SrtpKeys` + select `AeadAes256Gcm` (already supported by
//! the vendored `external/rtc/rtc-srtp`, profile `0x0008`). This codec only
//! *surfaces* `public_key` (on [`Signaling::public_key`]) so that work can consume
//! it; it implements none of the crypto.

#[cfg(feature = "signal")]
use prost::Message;

use crate::signaling::Signaling;
use crate::Error;

/// Hand-derived [`prost::Message`] mirrors of `proto/signal_signaling.proto`.
/// Tags and types are the on-wire contract — keep them in lockstep with the proto.
#[cfg(feature = "signal")]
mod proto {
    use prost::Message;

    /// `opaque` of `CallMessage.Offer`.
    #[derive(Clone, PartialEq, Message)]
    pub struct Offer {
        #[prost(message, optional, tag = "4")]
        pub v4: Option<ConnectionParametersV4>,
    }

    /// `opaque` of `CallMessage.Answer`.
    #[derive(Clone, PartialEq, Message)]
    pub struct Answer {
        #[prost(message, optional, tag = "4")]
        pub v4: Option<ConnectionParametersV4>,
    }

    /// `opaque` of one `CallMessage.IceUpdate`.
    #[derive(Clone, PartialEq, Message)]
    pub struct IceCandidate {
        #[prost(message, optional, tag = "2")]
        pub added_v3: Option<IceCandidateV3>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct IceCandidateV3 {
        /// Full SDP attribute line including the `candidate:` prefix.
        #[prost(string, optional, tag = "1")]
        pub sdp: Option<String>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct ConnectionParametersV4 {
        /// X25519 public key (32 bytes) — the DTLS-fingerprint replacement.
        #[prost(bytes = "vec", optional, tag = "1")]
        pub public_key: Option<Vec<u8>>,
        #[prost(string, optional, tag = "2")]
        pub ice_ufrag: Option<String>,
        #[prost(string, optional, tag = "3")]
        pub ice_pwd: Option<String>,
        #[prost(uint64, optional, tag = "5")]
        pub max_bitrate_bps: Option<u64>,
    }
}

#[cfg(feature = "signal")]
fn params_from(s: &Signaling) -> Result<proto::ConnectionParametersV4, Error> {
    // `public_key` is mandatory on the Signal path: without it Phase 2's DH has no
    // peer key. Surface the missing dependency loudly rather than emit a blob that
    // can't key SRTP. `fingerprint`/`setup`/`direction` have no ringrtc counterpart
    // and are intentionally dropped; `candidates` are trickled separately via
    // `encode_ice_candidate`, not carried in the offer/answer.
    let public_key = s
        .public_key
        .clone()
        .ok_or(Error::Proto("signal offer/answer requires public_key (X25519)"))?;
    Ok(proto::ConnectionParametersV4 {
        public_key: Some(public_key),
        ice_ufrag: Some(s.ice_ufrag.clone()),
        ice_pwd: Some(s.ice_pwd.clone()),
        max_bitrate_bps: None,
    })
}

#[cfg(feature = "signal")]
fn params_into(p: proto::ConnectionParametersV4) -> Signaling {
    // The Signal path negotiates no DTLS/SDP media params: default direction to
    // sendrecv, leave fingerprint/setup empty, and carry no candidates here (they
    // arrive via IceUpdate). `public_key` length is NOT validated here — that's the
    // DH layer's job (Phase 2); the codec accepts whatever bytes are on the wire.
    Signaling {
        ice_ufrag: p.ice_ufrag.unwrap_or_default(),
        ice_pwd: p.ice_pwd.unwrap_or_default(),
        fingerprint: String::new(),
        setup: String::new(),
        direction: "sendrecv".to_owned(),
        candidates: Vec::new(),
        public_key: p.public_key,
    }
}

/// Encode an [`Signaling`] as the `opaque` bytes of `CallMessage.Offer`.
#[cfg(feature = "signal")]
pub fn encode_offer(s: &Signaling) -> Result<Vec<u8>, Error> {
    Ok(proto::Offer { v4: Some(params_from(s)?) }.encode_to_vec())
}

/// Decode the `opaque` bytes of `CallMessage.Offer` into a [`Signaling`].
#[cfg(feature = "signal")]
pub fn decode_offer(b: &[u8]) -> Result<Signaling, Error> {
    let offer = proto::Offer::decode(b).map_err(|_| Error::Proto("offer opaque decode"))?;
    let v4 = offer.v4.ok_or(Error::Proto("offer missing v4 connection params"))?;
    Ok(params_into(v4))
}

/// Encode an [`Signaling`] as the `opaque` bytes of `CallMessage.Answer`.
#[cfg(feature = "signal")]
pub fn encode_answer(s: &Signaling) -> Result<Vec<u8>, Error> {
    Ok(proto::Answer { v4: Some(params_from(s)?) }.encode_to_vec())
}

/// Decode the `opaque` bytes of `CallMessage.Answer` into a [`Signaling`].
#[cfg(feature = "signal")]
pub fn decode_answer(b: &[u8]) -> Result<Signaling, Error> {
    let answer = proto::Answer::decode(b).map_err(|_| Error::Proto("answer opaque decode"))?;
    let v4 = answer.v4.ok_or(Error::Proto("answer missing v4 connection params"))?;
    Ok(params_into(v4))
}

/// Encode one ICE candidate as the `opaque` bytes of a `CallMessage.IceUpdate`.
///
/// `line` is a candidate in [`Signaling::candidates`] form — an `a=candidate:` line
/// **without** the `candidate:` prefix (matching [`Signaling::from_sdp`]). The wire
/// format carries the prefix, so we add it here.
#[cfg(feature = "signal")]
pub fn encode_ice_candidate(line: &str) -> Result<Vec<u8>, Error> {
    let msg = proto::IceCandidate {
        added_v3: Some(proto::IceCandidateV3 { sdp: Some(format!("candidate:{line}")) }),
    };
    Ok(msg.encode_to_vec())
}

/// Decode one `CallMessage.IceUpdate` `opaque` blob into a candidate line in
/// [`Signaling::candidates`] form (the `candidate:` prefix stripped).
///
/// Returns `Ok(None)` for a blob that carries no added candidate (e.g. a removal,
/// which wart-call does not yet act on).
#[cfg(feature = "signal")]
pub fn decode_ice_candidate(b: &[u8]) -> Result<Option<String>, Error> {
    let msg = proto::IceCandidate::decode(b).map_err(|_| Error::Proto("ice candidate decode"))?;
    Ok(msg
        .added_v3
        .and_then(|c| c.sdp)
        .map(|sdp| sdp.strip_prefix("candidate:").unwrap_or(&sdp).to_owned()))
}

#[cfg(all(test, feature = "signal"))]
mod tests {
    use super::*;

    fn sig(pk: Option<Vec<u8>>) -> Signaling {
        Signaling {
            ice_ufrag: "AB".to_owned(),
            ice_pwd: "CD".to_owned(),
            // These three have no ringrtc counterpart — set them to prove they're dropped.
            fingerprint: "sha-256 DE:AD".to_owned(),
            setup: "actpass".to_owned(),
            direction: "recvonly".to_owned(),
            candidates: vec!["1 1 udp 2130706431 10.0.0.1 5000 typ host".to_owned()],
            public_key: pk,
        }
    }

    #[test]
    fn offer_round_trip_preserves_creds_and_key() {
        let s = sig(Some(vec![0x01, 0x02]));
        let blob = encode_offer(&s).unwrap();
        let back = decode_offer(&blob).unwrap();
        assert_eq!(back.ice_ufrag, "AB");
        assert_eq!(back.ice_pwd, "CD");
        assert_eq!(back.public_key, Some(vec![0x01, 0x02]));
        // Dropped fields decode to documented defaults, NOT the originals.
        assert_eq!(back.fingerprint, "");
        assert_eq!(back.setup, "");
        assert_eq!(back.direction, "sendrecv");
        assert!(back.candidates.is_empty());
    }

    #[test]
    fn answer_round_trip_preserves_creds_and_key() {
        let s = sig(Some(vec![0xAA; 32]));
        let back = decode_answer(&encode_answer(&s).unwrap()).unwrap();
        assert_eq!(back.ice_ufrag, "AB");
        assert_eq!(back.ice_pwd, "CD");
        assert_eq!(back.public_key, Some(vec![0xAA; 32]));
    }

    #[test]
    fn ice_candidate_prefix_boundary() {
        let line = "1 1 udp 2130706431 10.0.0.1 5000 typ host";
        let blob = encode_ice_candidate(line).unwrap();
        // Round-trips to the same prefix-less line.
        assert_eq!(decode_ice_candidate(&blob).unwrap(), Some(line.to_owned()));
        // The WIRE form keeps the `candidate:` prefix (decode the raw proto to check).
        let raw = proto::IceCandidate::decode(&blob[..]).unwrap();
        assert_eq!(raw.added_v3.unwrap().sdp.unwrap(), format!("candidate:{line}"));
    }

    #[test]
    fn ice_candidate_without_added_is_none() {
        // An IceCandidate carrying no `added_v3` (e.g. a removal) → None.
        let blob = proto::IceCandidate { added_v3: None }.encode_to_vec();
        assert_eq!(decode_ice_candidate(&blob).unwrap(), None);
    }

    /// Golden bytes — locks wire compatibility with Signal/ringrtc WITHOUT a live
    /// client. Field tags must be 0x0A(public_key) / 0x12(ufrag) / 0x1A(pwd) inside
    /// an Offer whose `v4=4` message tag is 0x22. Derived from proto2 tag rules, not
    /// from ringrtc's (AGPL) crate. If this breaks, the on-wire format drifted.
    #[test]
    fn offer_golden_bytes() {
        let s = sig(Some(vec![0x01, 0x02])); // ufrag "AB", pwd "CD"
        let blob = encode_offer(&s).unwrap();
        #[rustfmt::skip]
        let expected: &[u8] = &[
            0x22, 0x0C,                   // Offer.v4 (tag 4, LEN), 12-byte ConnectionParametersV4
              0x0A, 0x02, 0x01, 0x02,     //   public_key (tag 1, LEN 2) = 01 02
              0x12, 0x02, 0x41, 0x42,     //   ice_ufrag  (tag 2, LEN 2) = "AB"
              0x1A, 0x02, 0x43, 0x44,     //   ice_pwd    (tag 3, LEN 2) = "CD"
        ];
        assert_eq!(blob, expected, "Signal opaque wire format drifted");
    }

    #[test]
    fn encode_offer_requires_public_key() {
        // The Phase 2 DH contract: no public key → explicit error, not a junk blob.
        assert_eq!(
            encode_offer(&sig(None)),
            Err(Error::Proto("signal offer/answer requires public_key (X25519)")),
        );
    }

    #[test]
    fn decode_rejects_garbage_and_missing_v4() {
        // Truncated protobuf (LEN field claims 5 bytes, 0 follow) → decode error.
        assert_eq!(decode_offer(&[0x22, 0x05]), Err(Error::Proto("offer opaque decode")));
        // An empty / no-v4 Offer (e.g. a future v5 we can't parse) → clean error.
        // (Empty bytes decode to a valid all-default Offer, i.e. v4 = None.)
        let empty_offer = proto::Offer { v4: None }.encode_to_vec();
        assert!(empty_offer.is_empty());
        for b in [&[][..], &empty_offer[..]] {
            assert_eq!(decode_offer(b), Err(Error::Proto("offer missing v4 connection params")));
        }
    }
}
