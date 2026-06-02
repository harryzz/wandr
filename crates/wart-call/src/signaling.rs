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
#[derive(Debug, Clone, Default)]
pub struct Signaling {
    pub ice_ufrag: String,
    pub ice_pwd: String,
    /// `sha-256 AB:CD:…`
    pub fingerprint: String,
    /// DTLS role: `actpass` (offer), `active` (answer→client), `passive`.
    pub setup: String,
    /// `a=candidate:…` values (without the `candidate:` prefix).
    pub candidates: Vec<String>,
}

impl Signaling {
    /// Marshal a minimal-but-valid WebRTC audio SDP (offer or answer).
    pub fn to_sdp(&self) -> String {
        let mut audio = MediaDescription::new_jsep_media_description("audio".to_owned(), vec![])
            .with_value_attribute("mid".to_owned(), "0".to_owned())
            .with_ice_credentials(self.ice_ufrag.clone(), self.ice_pwd.clone())
            .with_value_attribute("setup".to_owned(), self.setup.clone())
            .with_property_attribute("rtcp-mux".to_owned())
            .with_property_attribute("sendrecv".to_owned())
            .with_codec(
                OPUS_PAYLOAD_TYPE,
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
        Ok(Signaling {
            ice_ufrag: attr("ice-ufrag"),
            ice_pwd: attr("ice-pwd"),
            fingerprint: attr("fingerprint"),
            setup: attr("setup"),
            candidates: vec![], // trickle candidates arrive separately; full-SDP candidates TODO
        })
    }
}
