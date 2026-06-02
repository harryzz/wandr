# call-browser — wart-call ↔ real browser WebRTC test harness

The definitive "does wart-call speak real WebRTC?" check: a real browser
(Chrome/Firefox = Google **libwebrtc**) on one side, `wart-call` on the other.
Cross-implementation interop is already proven headless against the webrtc-rs
async stack (`../call-interop`); this is the libwebrtc confirmation — but it needs
*your* browser, so it's a harness you run.

## What it does

A tiny HTTP server serves a test page and relays SDP, with `wart-call` as the
answerer. The browser offers a **recvonly audio** call; wart-call answers,
connects (ICE + DTLS-SRTP over real UDP), and streams a 440 Hz Opus tone the
browser plays.

## Run

```bash
cargo run
# ▸ open  http://<this-machine-ip>:8088  in a browser on the same network
```
Open that URL, click **Start call**, and watch the page's state + log.

**Success** = the page shows `CONNECTED — wart-call ↔ browser ✓` and you hear a
440 Hz tone (and the terminal prints `✓ wart-call CONNECTED to the browser`).

## Two environment gotchas (read before blaming the code)

1. **Browser mDNS.** Chrome/Firefox hide the local IP behind `.local` mDNS ICE
   candidates by default, which wart-call can't resolve — so it gets no reachable
   remote candidate and never connects. Disable it:
   - Chrome: `chrome://flags/#enable-webrtc-hide-local-ips-with-mdns` → **Disabled**, relaunch.
   - Firefox: `about:config` → `media.peerconnection.ice.obfuscate_host_addresses` → **false**.
   Then the browser advertises real host candidates (its LAN IP) that wart-call reaches.

2. **Network reachability.** The browser must reach the machine running the
   harness over **UDP** at the advertised LAN IP. Run it on a host the browser can
   reach directly (a Linux box on the same LAN, or the same machine). **WSL is
   tricky** — its `172.x` network isn't reachable from a Windows/LAN browser
   without mirrored networking or port-forwarding; prefer a native host.

## What it proves / next

If it connects, wart-call's SDP/ICE/DTLS-SRTP interoperate with libwebrtc — the
real thing. If the browser rejects the answer or the audio doesn't render, the
browser console + the page log show why; the likely gaps are **strict-SDP
additions** libwebrtc wants in the answer that our minimal SDP omits (e.g.
`a=msid`/`a=ssrc` for the media track to render, exact `mid`/BUNDLE matching).
Connection (ICE+DTLS) is the core proof; audio *rendering* may need those lines —
paste the browser's errors and they're quick to add.

The headless interop (`../call-interop`) already validates ICE + DTLS-SRTP
against an independent stack, so this harness is mostly about surfacing any
libwebrtc-specific SDP strictness.
