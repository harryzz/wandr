//! Signaling — the SDP offer/answer carrying the parameters two peers must agree
//! on out-of-band: ICE ufrag/pwd, the DTLS-cert fingerprint, the Opus rtpmap, and
//! ICE candidates. WebRTC-specific (a SIP/Jingle backend would swap this module).
//!
//! Device-verified as repros/call-signaling-sdp.

use std::io::Cursor;

use rtc_sdp::description::media::MediaDescription;
use rtc_sdp::description::session::SessionDescription;

use crate::{Error, OPUS_PAYLOAD_TYPE};

/// The signaling parameters an SDP carries for one audio m-section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signaling {
    pub ice_ufrag: String,
    pub ice_pwd: String,
    /// `sha-256 AB:CD:…`
    pub fingerprint: String,
    /// DTLS role: `actpass` (offer), `active` (answer→client), `passive`.
    pub setup: String,
    /// Media direction: `sendrecv` / `sendonly` / `recvonly` / `inactive`.
    /// An answer must be the reverse-compatible of the offer (e.g. a `recvonly`
    /// offer requires a `sendonly` answer — libwebrtc rejects a `sendrecv` one).
    pub direction: String,
    /// `a=candidate:…` values (without the `candidate:` prefix).
    pub candidates: Vec<String>,
    /// The Opus dynamic payload type for this m-section. On the WebRTC-native path
    /// it's the NEGOTIATED value (parsed from the peer's SDP rtpmap by `from_sdp`,
    /// emitted by `to_sdp`); on the Signal path it's ringrtc's fixed value. The
    /// session adopts the remote's value so we send/decode on the PT the peer uses.
    pub audio_pt: u8,
    /// X25519 public key for Signal's DH-SRTP keying
    /// (ringrtc `ConnectionParametersV4.public_key`, the DTLS-fingerprint
    /// replacement). `None` on the WebRTC-native/DTLS path (`to_sdp`/`from_sdp`
    /// ignore it); `Some(32 bytes)` on the Signal path (`crate::signal`).
    pub public_key: Option<Vec<u8>>,
}

impl Default for Signaling {
    fn default() -> Self {
        Signaling {
            ice_ufrag: String::new(),
            ice_pwd: String::new(),
            fingerprint: String::new(),
            setup: String::new(),
            direction: String::new(),
            candidates: Vec::new(),
            audio_pt: OPUS_PAYLOAD_TYPE,
            public_key: None,
        }
    }
}

/// The answer direction for a given offer direction (RFC 3264).
pub fn reverse_direction(offer: &str) -> &'static str {
    match offer {
        "recvonly" => "sendonly",
        "sendonly" => "recvonly",
        "inactive" => "inactive",
        _ => "sendrecv",
    }
}

impl Signaling {
    /// Marshal a minimal-but-valid WebRTC audio SDP (offer or answer).
    pub fn to_sdp(&self) -> String {
        let mut audio = MediaDescription::new_jsep_media_description("audio".to_owned(), vec![])
            .with_value_attribute("mid".to_owned(), "0".to_owned())
            .with_ice_credentials(self.ice_ufrag.clone(), self.ice_pwd.clone())
            .with_value_attribute("setup".to_owned(), self.setup.clone())
            .with_property_attribute("rtcp-mux".to_owned())
            .with_property_attribute(
                if self.direction.is_empty() { "sendrecv".to_owned() } else { self.direction.clone() },
            )
            .with_codec(
                self.audio_pt,
                "opus".to_owned(),
                crate::SAMPLE_RATE,
                2,
                "minptime=10;useinbandfec=1".to_owned(),
            );
        if let Some((alg, val)) = self.fingerprint.split_once(' ') {
            audio = audio.with_fingerprint(alg.to_owned(), val.to_owned());
        }
        for c in &self.candidates {
            audio = audio.with_candidate(c.clone());
        }
        let sdp = SessionDescription::new_jsep_session_description(false)
            .with_value_attribute("group".to_owned(), "BUNDLE 0".to_owned())
            .with_media(audio);
        format!("{sdp}")
    }

    /// Parse the peer's SDP, extracting the audio m-section's signaling params.
    pub fn from_sdp(sdp: &str) -> Result<Self, Error> {
        let mut cursor = Cursor::new(sdp.as_bytes());
        let parsed = SessionDescription::unmarshal(&mut cursor)
            .map_err(|_| Error::Sdp("unmarshal"))?;
        let media = parsed
            .media_descriptions
            .iter()
            .find(|m| m.media_name.media == "audio")
            .ok_or(Error::Sdp("no audio m-section"))?;

        let attr = |k: &str| media.attribute(k).flatten().unwrap_or("").to_owned();
        // ALL candidate lines (a peer offers several — IPv4/IPv6/host/srflx);
        // `attribute()` only returns the first, so iterate the raw attributes.
        let candidates: Vec<String> = media
            .attributes
            .iter()
            .filter(|a| a.key == "candidate")
            .filter_map(|a| a.value.clone())
            .collect();
        let direction = ["sendrecv", "sendonly", "recvonly", "inactive"]
            .into_iter()
            .find(|d| media.has_attribute(d))
            .unwrap_or("sendrecv")
            .to_owned();
        // The Opus payload type from `a=rtpmap:<PT> opus/48000/2` — the PT the peer
        // sends/expects. Honor it rather than assuming our default, so we interop
        // with any peer's dynamic-PT choice. Falls back to our default if absent.
        let audio_pt = media
            .attributes
            .iter()
            .filter(|a| a.key == "rtpmap")
            .filter_map(|a| a.value.as_deref())
            .find(|v| v.to_ascii_lowercase().contains("opus"))
            .and_then(|v| v.split_whitespace().next())
            .and_then(|pt| pt.parse::<u8>().ok())
            .unwrap_or(OPUS_PAYLOAD_TYPE);
        Ok(Signaling {
            ice_ufrag: attr("ice-ufrag"),
            ice_pwd: attr("ice-pwd"),
            fingerprint: attr("fingerprint"),
            setup: attr("setup"),
            direction,
            candidates,
            audio_pt,
            public_key: None, // SDP path carries no X25519 key (Signal-only; see crate::signal)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_pt_round_trips_through_sdp() {
        // A non-default PT survives to_sdp → from_sdp (interop with a peer that
        // picks its own dynamic PT).
        let mut s = Signaling { audio_pt: 96, ..Default::default() };
        s.fingerprint = "sha-256 AB:CD".to_owned();
        let parsed = Signaling::from_sdp(&s.to_sdp()).unwrap();
        assert_eq!(parsed.audio_pt, 96);
    }

    #[test]
    fn default_offer_advertises_102() {
        // Our default offer uses ringrtc's Opus PT, and it parses back as 102.
        let parsed = Signaling::from_sdp(&Signaling::default().to_sdp()).unwrap();
        assert_eq!(parsed.audio_pt, OPUS_PAYLOAD_TYPE);
        assert_eq!(OPUS_PAYLOAD_TYPE, 102);
    }
}
