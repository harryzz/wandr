# call-browser — wandr-call ↔ real browser WebRTC test harness

The definitive "does wandr-call speak real WebRTC?" check: a real browser
(Chrome/Firefox = Google **libwebrtc**) on one side, `wandr-call` on the other.
Cross-implementation interop is already proven headless against the webrtc-rs
async stack (`../call-interop`); this is the libwebrtc confirmation — but it needs
*your* browser, so it's a harness you run.

## What it does

A tiny HTTP server serves a test page and relays SDP, with `wandr-call` as the
answerer. The browser offers a **recvonly audio** call; wandr-call answers,
connects (ICE + DTLS-SRTP over real UDP), and streams a 440 Hz Opus tone the
browser plays.

## Run

```bash
cargo run
# ▸ open  http://<this-machine-ip>:8088  in a browser on the same network
```
Open that URL, click **Start call**, and watch the page's state + log.

**Success** = the page shows `CONNECTED — wandr-call ↔ browser ✓` and you hear a
440 Hz tone (and the terminal prints `✓ wandr-call CONNECTED to the browser`).

## Two environment gotchas (read before blaming the code)

1. **Browser mDNS.** Chrome/Firefox hide the local IP behind `.local` mDNS ICE
   candidates by default, which wandr-call can't resolve — so it gets no reachable
   remote candidate and never connects. Disable it:
   - Chrome: `chrome://flags/#enable-webrtc-hide-local-ips-with-mdns` → **Disabled**, relaunch.
   - Firefox: `about:config` → `media.peerconnection.ice.obfuscate_host_addresses` → **false**.
   Then the browser advertises real host candidates (its LAN IP) that wandr-call reaches.

2. **Network reachability.** The browser must reach the machine running the
   harness over **UDP** at the advertised LAN IP. Run it on a host the browser can
   reach directly (a Linux box on the same LAN, or the same machine). **WSL is
   tricky** — its `172.x` network isn't reachable from a Windows/LAN browser
   without mirrored networking or port-forwarding; prefer a native host.

## Result — VERIFIED ✅ (2026-06-02)

wandr-call connected to a real browser (Google libwebrtc): `CONNECTED — wandr-call
↔ browser ✓`, and the browser **played the Opus tone** wandr-call streamed over
SRTP. So wandr-call's SDP/ICE/DTLS-SRTP/Opus interoperate with the actual
reference WebRTC implementation, media included. (The audio rendered without
`a=msid`/`a=ssrc` — libwebrtc accepted the unsignaled stream.)

The only fix the browser demanded was mirroring the offer's media **direction**
in the answer (a `recvonly` offer needs a `sendonly` answer — RFC 3264; handled
in `wandr_call::signaling`). The headless `../call-interop` validates the same
against an independent stack.
