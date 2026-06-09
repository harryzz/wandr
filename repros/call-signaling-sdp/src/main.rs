//! Call engine — Stage 3: signaling (SDP), in a wasm32-wasip2 guest.
//!
//! The transport + media stages produce a set of parameters that two peers must
//! agree on out-of-band before the connection forms: ICE ufrag/pwd, the
//! DTLS-certificate fingerprint, the Opus rtpmap, and ICE candidates. WebRTC
//! carries them in an **SDP offer/answer**. This proves a guest can build that
//! payload (`rtc-sdp` marshal), parse it back (unmarshal), and extract the
//! fields — the signaling round-trip.
//!
//! Run on-device: `wandr-host --run-once wandr.probe.sdp`.

use std::io::Cursor;

use rtc_sdp::description::media::MediaDescription;
use rtc_sdp::description::session::SessionDescription;

fn main() {
    // Params the other stages produce (ICE creds from the agent, fingerprint from
    // the DTLS cert, Opus from the codec, a candidate from ICE gathering).
    let ufrag = "ufrAgentA";
    let pwd = "passwordApasswordApasswordA00";
    let fingerprint = "12:34:56:78:9A:BC:DE:F0:12:34:56:78:9A:BC:DE:F0:\
                       12:34:56:78:9A:BC:DE:F0:12:34:56:78:9A:BC:DE:F0";
    let candidate = "1 1 udp 2130706431 192.168.1.5 50000 typ host";

    // Build the WebRTC audio m-section.
    let audio = MediaDescription::new_jsep_media_description("audio".to_owned(), vec![])
        .with_value_attribute("mid".to_owned(), "0".to_owned())
        .with_ice_credentials(ufrag.to_owned(), pwd.to_owned())
        .with_fingerprint("sha-256".to_owned(), fingerprint.to_owned())
        .with_value_attribute("setup".to_owned(), "actpass".to_owned())
        .with_property_attribute("rtcp-mux".to_owned())
        .with_property_attribute("sendrecv".to_owned())
        .with_codec(111, "opus".to_owned(), 48000, 2, "minptime=10;useinbandfec=1".to_owned())
        .with_candidate(candidate.to_owned());

    let offer = SessionDescription::new_jsep_session_description(false)
        .with_value_attribute("group".to_owned(), "BUNDLE 0".to_owned())
        .with_media(audio);

    let sdp_text = format!("{offer}"); // Display = SDP marshal
    println!("--- generated SDP offer ({} bytes) ---", sdp_text.len());
    print!("{sdp_text}");
    println!("---");

    // Parse it back (the receiving peer's view).
    let mut cursor = Cursor::new(sdp_text.as_bytes());
    let parsed = SessionDescription::unmarshal(&mut cursor).expect("SDP unmarshal");
    let media = parsed.media_descriptions.first().expect("audio m-section");

    let attr = |k: &str| media.attribute(k).flatten().unwrap_or("");
    let got_ufrag = attr("ice-ufrag");
    let got_fp = attr("fingerprint");
    let got_setup = attr("setup");
    let got_rtpmap = attr("rtpmap");
    println!("[sdp] parsed m={} formats={:?}", media.media_name.media, media.media_name.formats);
    println!("[sdp] ice-ufrag={got_ufrag} setup={got_setup}");
    println!("[sdp] fingerprint={}", &got_fp[..got_fp.len().min(24)]);
    println!("[sdp] rtpmap={got_rtpmap}");
    println!("[sdp] candidate present={}", media.has_attribute("candidate"));

    // Verify the round-trip preserved every signaling field.
    assert_eq!(media.media_name.media, "audio");
    assert_eq!(got_ufrag, ufrag, "ice-ufrag survived");
    assert!(got_fp.contains("sha-256"), "fingerprint survived");
    assert_eq!(got_setup, "actpass", "DTLS setup role survived");
    assert!(media.media_name.formats.contains(&"111".to_owned()), "Opus PT survived");
    assert!(got_rtpmap.contains("opus/48000/2"), "Opus rtpmap survived");
    assert!(media.has_attribute("candidate"), "ICE candidate survived");
    println!("SIGNALING OK — WebRTC SDP offer built + parsed + all fields extracted on wasm32-wasip2");
}
