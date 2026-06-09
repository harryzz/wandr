# wasi-udp-probe — UDP transport de-risk for the WebRTC call engine

The sans-IO `rtc` stack ([`../webrtc-rs-wasip2`](../webrtc-rs-wasip2)) leaves UDP
IO to the embedder. This probe answers: **can a wasm32-wasip2 guest do UDP
send/recv through wandr-host?** Answer: **yes, via `wasi:sockets` — no custom host
interface needed.**

## Finding

wandr-host already wires `wasi:sockets` (`wasmtime_wasi::p2::add_to_linker_sync`,
v45) and `signal_tls::grant_network()` does `inherit_network()` +
`allow_ip_name_lookup(true)`. So a guest uses `std::net::UdpSocket` directly; the
sans-IO `rtc` engine would drive its IO this way (poll_write → send_to,
recv_from → handle_read), using `set_nonblocking(true)` + poll.

Note: wasip2 std has **no `SO_RCVTIMEO`** (`set_read_timeout` → `ENOPROTOOPT`);
use `set_nonblocking` + poll instead (what the sans-IO loop does anyway).

## What it does
1. **Loopback** — two UDP sockets on 127.0.0.1, send + recv. Proves the
   wasi-sockets UDP API (bind/send_to/recv_from) works for the guest.
2. **STUN** — a hand-rolled Binding Request to `stun.l.google.com:19302`, parse
   XOR-MAPPED-ADDRESS. Proves outbound internet UDP + DNS + yields our
   server-reflexive address (the srflx ICE candidate — the first thing ICE needs).

## Run

```bash
# build
cargo build --target wasm32-wasip2 --release

# desktop (wasmtime 45, same as the host)
wasmtime run -S inherit-network -S allow-ip-name-lookup \
    target/wasm32-wasip2/release/wasi-udp-probe.wasm

# on-device through wandr-host (package as a wasi:cli/command wandrpkg, then):
wandr-host --install <wandrpkg>           # app_id wandr.probe.udp
wandr-host --run-once wandr.probe.udp
```

## Result (2026-06-02) — desktop AND device, both green
```
UDP LOOPBACK OK — wasi-sockets UDP works for the guest
STUN response: 32 bytes from 74.125.250.129:19302
UDP OUTBOUND OK — STUN server-reflexive address = 77.70.64.156:46182
```
Device run via `wandr-host --run-once wandr.probe.udp` (Pixel 2 XL, WiFi);
`run_once: call_run returned Ok — guest exited cleanly`.

## Implication for the call engine — UDP glue is essentially done
- **No custom UDP host import needed** — the guest owns its sockets via wasi-sockets.
- **srflx candidate** (the hard NAT case) works — proven here.
- **host candidates** (LAN IPs) need no interface enumeration either: the guest
  can bind + "connect" a UDP socket to a public IP and read `local_addr()` to get
  the LAN IP for that route (the standard trick), all within wasi-sockets.
- **TURN relay** (symmetric NAT) is the only piece needing external infra (a TURN
  server), and `rtc-turn` is sans-IO over the same UDP.

So the sans-IO `rtc` engine + `std::net::UdpSocket` is a complete transport for
the guest. Remaining call-engine work is now just **Opus** (libopus→wasip2) +
wiring the engine to our audio capture/playback + the host crypto interface (perf).
