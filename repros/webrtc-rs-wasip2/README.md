# webrtc-rs (`rtc`) on wasm32-wasip2 — de-risk spike + rtc-ice mDNS-optional fork

Evaluating the **sans-IO** webrtc-rs crate (https://github.com/webrtc-rs/rtc) as
the call-engine for a wandr guest (Signal VoIP / generic WebRTC). Date: 2026-06-02,
`rtc` master, rustc 1.95, target `wasm32-wasip2`. Full design context:
`.claude/memory/project_crypto_hw_offload.md`.

## Spike result (unforked)

Built **clean** for wasm32-wasip2 as-is:
- `rtc-srtp` (pure RustCrypto)
- `rtc-dtls` — **`ring 0.17.14` + `rustls` + `rcgen` all compile** (the old
  "ring blocks wasm" assumption is outdated; no de-ring needed)
- `rtc-stun`, `sansio`, `rtc-shared` (with `default-features=false`)

**Only blocker:** `rtc-mdns` (pulls `socket2` + `tokio` for real multicast UDP —
inherently IO, doesn't build for wasip2), and `rtc-ice` *only* because it
hard-depends on `rtc-mdns`. mDNS ICE candidates are *optional* WebRTC
functionality (they hide the local IP), and candidate gathering / sockets are
exactly what the sans-IO design leaves to the embedder.

## The fork (`wandr-rtc.patch` — was `rtc-ice-mdns-optional.patch`)

The patch has since grown beyond mDNS: it also carries the task-16 ICE
self-select/diagnostics and the task-93 rtc-srtp `external-aead` feature (see
`tools/scripts/patch-rtc.sh` for the full inventory). The mDNS part:

Makes `mdns` an **optional, default-on** Cargo feature in `rtc-ice` and
`#[cfg(feature = "mdns")]`-gates every reference to the `rtc-mdns` *types*
(`Mdns`, `QueryId`, `MdnsEvent`, `MDNS_PORT`, `create_multicast_dns`). The agent
already treats `mdns: Option<Mdns>` as always-optional (every use is
`if let Some(mdns_conn)`), so the runtime already no-ops without it — only the
type references needed gating. `MulticastDnsMode` + `generate_multicast_dns_name`
stay (pure, no rtc-mdns dep). 4 files, +44/−5.

Apply against a fresh clone:
```
git clone https://github.com/webrtc-rs/rtc && cd rtc
git apply /path/to/wandr-rtc.patch
```

## Verified (all green)
```
cargo build -p rtc-ice --no-default-features --target wasm32-wasip2          # ✅ clean
cargo build -p rtc-dtls -p rtc-srtp -p rtc-ice --no-default-features \
            --target wasm32-wasip2                                           # ✅ full sans-IO stack
cargo build -p rtc-ice                                                       # ✅ native default (mDNS unbroken)
```

## What's still needed for a working call engine (known/bounded)
1. **Host UDP transport** — sans-IO → the host opens the sockets and pumps
   packets into the guest's poll/handle loop (wasi:sockets/udp or a host import).
2. **Local ICE candidates** — the host knows its own IPs; feed them in (rtc-ice
   uses `rtc-shared` with `default-features=false`, so it does NOT do interface
   enumeration itself — the embedder supplies candidates. This is the sans-IO
   contract working *for* us).
3. **Opus codec** — not bundled by webrtc-rs; compile `libopus` → wasip2 (with
   `+simd128` for NEON) or a pure-Rust decoder.
4. **Hot-path crypto** — works in-guest (RustCrypto), but SRTP AES-GCM wants
   host-side ARMv8 hardware AES for throughput/battery — see the crypto memo.

## Caveat
Signal calls use **ringrtc** (Signal's libwebrtc wrapper) + Signal's calling
service, not plain WebRTC → `rtc` suits a generic/custom call feature better than
drop-in Signal-peer interop.
