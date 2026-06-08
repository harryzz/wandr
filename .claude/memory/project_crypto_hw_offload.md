---
name: project_crypto_hw_offload
description: "Roadmap decision: offload hot-path crypto (SRTP AES-GCM, DTLS, etc.) from the wasm guest to a HOST WIT interface backed by ARMv8 hardware AES. Not built — design note for the call engine."
metadata: 
  node_type: memory
  type: project
  originSessionId: 981b38b9-858e-4c22-b30d-89c53be34749
---

**Roadmap decision (2026-06-02, user-noted): hot-path crypto should run HOST-side
on ARMv8 hardware, not in the wasm guest.** Surfaced while evaluating
webrtc-rs/`rtc` for the call engine ([[reference_audio_policy_calls]],
[[project_arbiter_audio]]).

**The core fact: WASM cannot touch crypto hardware.** A wasm32-wasip2 guest is
sandboxed bytecode with no path to the ARMv8 AES/PMULL/SHA instructions. So
RustCrypto's `aes`/`aes-gcm` running INSIDE the guest is SOFTWARE AES (constant-
time bitsliced, ~5–10× slower + more battery than hardware). For SRTP that's the
hot path (AES-GCM per RTP packet, ~50 audio pkt/s now, far more with video) — the
worst place for in-wasm crypto.

**Mechanism: a host WIT interface the guest imports** (e.g. `wart:crypto` —
AES-GCM seal/open, AES-CTR, SHA-256, HMAC, P-256 ECDH/ECDSA). The host does the
native crypto; on our aarch64 host RustCrypto's `aes` auto-detects + uses the
ARMv8 Crypto Extensions at runtime (so do aws-lc-rs/ring) — so "offload to
hardware AES" is free once the call runs host-side. Pixel 2 XL / Snapdragon 835
has the full ARMv8.0 crypto set (AES/SHA1/SHA2/PMULL). Precedent: `wasi:tls`
(task 66) already runs host-side → already gets hardware AES. Standard pattern:
the `wasi:crypto` proposal (experimental wasmtime support).

**Why it's elegant — solves two problems at once:** it's the BETTER answer to
the `rtc-dtls` `ring` wasm-blocker. Instead of de-ringing into slow in-wasm
pure-Rust crypto, route the heavy crypto (AES-GCM/CTR, DTLS PRF, ECDH) to the
host interface — the ring problem AND the perf/battery problem dissolve together.

**TWO-TIER crypto design (device-confirmed 2026-06-02).** Android HAS a native
TEE crypto binder: `android.system.keystore2.IKeystoreService/default` (Keystore
2.0, + IKeystoreAuthorization/Maintenance/LegacyKeystore), backed by KeyMint →
QSEE/TrustZone on the msm8998/taimen (no Titan M / StrongBox here — TEE only).
But it's for KEY PROTECTION, not bulk throughput — every op is a binder→TEE
round-trip (context switch + contention), so it's wrong for SRTP (encrypting ~50
pkt/s through the TEE would dominate). Its op API IS streaming (createOperation→
update→finish) but each update still hops to the TEE → fine for "decrypt a file
once", wasteful for real-time media. So:
- **Media/bulk symmetric (SRTP AES-GCM)** → ARMv8 AES in-process (the host
  `wart:crypto` interface). No IPC, hardware instructions. WINS the media path.
- **Key protection** → `IKeystoreService` over rsbinder (like AAudio/AudioPolicy):
  Signal long-term identity keys, the DTLS long-term key, signing, attestation,
  key-wrapping — keys that benefit from TEE isolation. Caveats: keystore
  namespaces keys by CALLER uid/domain (root gets its own namespace — can make/use
  its OWN keys, can't read another app's, by design); KeyDescriptor/KeyParameters
  parcelables are moderately involved; check SELinux keystore access from our
  domain (a read-only probe de-risks it, like the audio probe). The DTLS-SRTP
  ephemeral self-signed cert key can stay in normal world (per-session); the
  long-term identity keys are what's worth TEE-protecting.

**`--no-art` SURVIVAL + the Keymaster-3.0 correction (device-checked 2026-06-08).**
On taimen, in a live `--no-art` stack (system_server gone), ALL crypto AIDL + HAL
services **survive** — standalone init daemons / vendor hwbinder HALs, none hosted in
system_server (same reason audioserver/sensorservice survive):
- AIDL (alive): `android.system.keystore2.IKeystoreService/default` (keystore2,
  `class early_hal`, pid 745) + companions `android.security.{authorization,compat,
  legacykeystore,maintenance,metrics}`; `android.security.identity.ICredentialStoreFactory`
  (credstore 1224); `android.service.gatekeeper.IGateKeeperService` (gatekeeperd 1421);
  `android.hardware.drm.IDrmFactory/clearkey`.
- HAL backends (alive): **`android.hardware.keymaster@3.0::IKeymasterDevice/default`**
  (pid 749 — the TEE/QSEE key root); `android.hardware.gatekeeper@1.0` (748);
  `android.hardware.drm@1.0–1.3::I{Crypto,Drm}Factory/widevine` (1122, OEMCrypto in TEE).
- **CORRECTION to the two-tier text above:** this SoC is **Keymaster 3.0 _HIDL_**, NOT
  KeyMint AIDL (taimen predates KeyMint). Callers use keystore2's AIDL (`IKeystoreService`),
  which wraps the HIDL keymaster HAL via `android.security.compat` (`IKeystoreCompatService`).
- **The SRTP hot path needs NONE of the above.** `/proc/cpuinfo` Features =
  `aes pmull sha1 sha2 crc32` (full ARMv8 Crypto Extensions). Bulk SRTP AES-GCM runs
  IN-PROCESS in the host (RustCrypto auto-uses these CPU instructions) — no AIDL/HAL/IPC;
  survives `--no-art` because it's the CPU. Keystore/keymaster is ONLY key protection /
  attestation / DRM, never bulk media.
- STILL UNPROVEN: reachability ≠ usable from OUR caller. Under `--no-art` we're root in
  the `su` SELinux domain; whether that domain can keystore2 `createOperation` is a
  separate namespace/SELinux question (read-only probe pending). Presence + survival
  confirmed; a successful key op from our domain is not.

**OFFLOAD ARCHITECTURE — three distinct layers (don't conflate; Signal video+audio).**
"Offload heavy media to the host, guest imports a WIT" is the right shape, but it's
THREE separate things with different host backends:
1. **Crypto (SRTP AES-GCM, DTLS PRF/ECDH)** → host RustCrypto on ARMv8 HW AES. Pure-Rust,
   NO AIDL, NO HAL — just CPU instructions. ✅ this is the layer where "Rust native, no
   AIDL, just a WIT the guest imports" is exactly correct (`wart:crypto`, not built).
2. **Video codec — BIDIRECTIONAL, both need HW offload** (a wasm guest can't do real-time
   VP8/VP9 either way). Host drives the **hardware MediaCodec** (NDK `AMediaCodec` → Codec2
   HAL `android.hardware.media.c2`, survives `--no-art`); guest only does crypto + RTP
   framing. NOT pure-Rust, but from the GUEST's view still just a WIT import.
   - **OUT (our camera→peer):** YUV → HW VP8 **ENCODE** → SRTP. ✅ proven (`--probe-video`
     17.4fps HW VP8 under --no-art, task 93/95).
   - **IN (peer→us):** SRTP-open → RTP depacketize → bitstream → HW **DECODE** → YUV →
     display. 🔲 NOT built (the "decode path = remaining" piece). Codec nego offers VP9
     then VP8 (`signal/mod.rs`) → **verify the device's MediaCodec has a HW VP9 decoder**
     (VP8 certain on msm8998; VP9 to confirm).
   - Architecture decision before the `wart:video` WIT: incoming decoded frame =
     **decode-to-surface** (host composites with guest UI, zero copy, best for 30fps,
     video plane outside guest skia) vs **decode-to-buffer** (host→guest WIT returns
     pixels, cleaner compositing, per-frame copy at video rates). Decode-to-surface is the
     usual call winner; it decides whether `decode-frame` returns pixels or just composites.
3. **Audio codec (Opus)** → already PURE-RUST and fast enough to run **IN the guest**
   (restsend/opus-rs, ~40× real-time, device-verified); audio I/O (mic + AAudio playback)
   already host-side. So audio needs NO codec offload — only its SRTP shares layer 1.

WIT direction: the **host implements**, the guest **imports** (component-model: host-provided
imports — "exported in the guest" = the guest world lists them as imports). Net: crypto =
pure-Rust HW-AES (no AIDL ✓); video = host MediaCodec/Codec2 HAL (survives --no-art); audio
codec = stays in-guest.

**SIGNAL IS IN-GUEST (corrected 2026-06-08, user + code-checked) — offload applies NOW.**
Earlier notes (incl. [[project_wart_call]]) said "Signal = ringrtc, C++, separate." WRONG:
`crates/wart-call` is a **pure-Rust reimplementation of ringrtc's V4 wire protocol**
(V4 keying, RTP-data control channel PT101/SSRC 0xD, connection-params protobuf, VP9/VP8
codec nego) running **in the wasm guest** — wire-compatible with a real ringrtc peer, NOT
linked libwebrtc/ringrtc C++. So the SRTP encrypt/decrypt runs IN-WASM: Signal calls use
**`ProtectionProfile::AeadAes256Gcm`** (`transport.rs:468`, `Keying::Signal`; keys from
X25519-DH + HKDF-SHA256, no DTLS) via `rtc_srtp::Context::{encrypt_rtp,decrypt_rtp}`
(`media.rs:158/183/195`) = **software AES-256-GCM per RTP packet in the sandbox**. ⇒ the
layer-1 host crypto offload is REAL and high-value for Signal right now (not hypothetical):
a `wart:crypto` WIT exposing AES-256-GCM seal/open that the guest imports → host RustCrypto
on ARMv8 HW AES. Cleanest granularity = per-packet seal/open, keep SRTP framing (ROC/replay/
key-derivation in `rtc_srtp::Context`) in-guest, offload only the AEAD primitive. (WebRTC-
native path uses `Aes128CmHmacSha1_80`.) See [[project_wart_call]], [[project_artless_camera]].

**WASM SIMD (simd128) — helps codecs/DSP, NOT AES.** Already supported host-side:
our wasmtime Config enables gc/function-references/exceptions and `wasm_simd` is
default-ON (we don't disable it); AOT `wasmtime compile` lowers wasm `v128` →
ARM NEON on the device. Enabling = a GUEST build flag (`-C target-feature=
+simd128`, Rust; Kotlin/Wasm likely doesn't emit SIMD yet → a Rust-guest lever).
BUT wasm SIMD has NO AES opcode (no AESE/AESMC equivalent) — wasmtime emits NEON
but never the ARM crypto-extension instructions. So SIMD only gives faster
SOFTWARE AES (~2–4× scalar), still far below native HW AES → does NOT change the
host-offload conclusion for SRTP. Where SIMD IS a real win (in-guest, no special
instructions needed): Opus encode/decode (libopus SIMD paths → +simd128 → NEON),
audio DSP (resample/mix/jitter/AGC/NS), video later (YUV/motion). Relaxed-SIMD
(FMA) is a further opt-in for audio/video math. Net: AES→host HW instructions;
codec+DSP→guest +simd128.

**Design constraints to honor when built:**
- **Trust boundary**: host-side crypto means SRTP media keys + plaintext cross
  the guest→host WIT boundary. Acceptable for a runtime that OWNS the device, but
  state it explicitly (deliberate widening of guest→host trust).
- **Granularity**: per-packet seal/open (AES-GCM of a ~1 KB packet) so the WIT
  call overhead amortizes; avoid a chatty byte-level API.

**webrtc-rs/`rtc` fit (evaluated):** sans-IO design = good fit for our single-
thread wasm guest + host-driven IO (the async `webrtc` crate hard-deps tokio →
does NOT fit). SRTP/RTP/RTCP/SDP/STUN/ICE are pure-Rust → wasip2-clean. Only
`rtc-dtls` pulls `ring` (via `rcgen` cert-gen + `rustls`) — the one blocker,
contained + swappable (or host-offloaded per above). Still need: UDP transport
(sans-IO → host import or wasi:sockets/udp) + Opus codec (compile libopus to
wasip2; not bundled). Audio I/O is DONE (mic capture + AAudio playback, this
session). CAVEAT: Signal calls use ringrtc, not plain WebRTC → `rtc` suits a
generic/custom call feature better than drop-in Signal-peer interop.

**SPIKE RUN 2026-06-02 (rtc master, rustc 1.95, wasm32-wasip2) — results:**
- ✅ BUILD CLEAN: rtc-srtp, **rtc-dtls** (ring 0.17.14 + rustls + rcgen ALL
  compile), rtc-stun, sansio, rtc-shared (with default-features=false).
- ❌ BLOCKED: rtc-mdns (socket2 + tokio — mDNS = real UDP multicast, inherently
  IO) → and rtc-ice ONLY because it hard-deps rtc-mdns (its other deps —
  shared-no-ifaces, sansio, stun — are all clean) → and the top `rtc` facade via
  ice→mdns.
- **REVERSAL: `ring` is NOT a wasip2 blocker anymore** (ring 0.17.14 builds for
  wasip2). So rtc-dtls compiles AS-IS; no de-ring needed. The host crypto-offload
  now stands purely as a PERF/battery choice for the SRTP hot path, NOT a
  build necessity.
- The only real friction is the inherently-IO part of ICE (mdns sockets +
  interface-enum `ifaces`) — exactly what sans-IO leaves to the embedder.
- PATH: fork rtc-ice to make `mdns` optional (cfg-gate the mdns module + agent
  mDNS gathering — 81 refs, woven into agent_config; a real but bounded fork) →
  full sans-IO stack builds for wasip2. Then provide UDP + local candidates
  host-side (host opens sockets + knows its IPs → feeds the guest state machine).
  Opus (libopus→wasip2 +simd128) still separate. Spike clone was /tmp/rtc-spike.
- **FORK DONE (2026-06-02): rtc-ice mDNS-optional.** Made `mdns` an optional
  default-on Cargo feature + `#[cfg(feature="mdns")]`-gated every rtc-mdns TYPE
  ref (Mdns/QueryId/MdnsEvent/MDNS_PORT/create_multicast_dns); the agent already
  treats mdns as always-Option so only type refs needed gating. 4 files, +44/−5.
  VERIFIED ALL GREEN: `rtc-ice --no-default-features --target wasm32-wasip2`
  (clean) + the full `rtc-dtls+rtc-srtp+rtc-ice` sans-IO stack on wasip2 + native
  default (mDNS unbroken). **The entire WebRTC protocol/crypto core now builds for
  our wasm32-wasip2 guest.** Patch + README saved: `repros/webrtc-rs-wasip2/`.
  Remaining = host UDP+candidates glue + Opus (libopus→wasip2). NOT committed to
  the wart repo as code — it's an upstream-fork patch in repros/.
- **UDP TRANSPORT GLUE: DONE/de-risked (2026-06-02, device-verified).** TURNS OUT
  THERE'S NO CUSTOM UDP HOST CODE TO BUILD — wart-host already wires wasi:sockets
  (wasmtime-wasi p2 v45) + `signal_tls::grant_network()` does inherit_network() +
  allow_ip_name_lookup(true). A wasm32-wasip2 guest uses `std::net::UdpSocket`
  directly; the sans-IO rtc engine drives IO via set_nonblocking+poll (wasip2 std
  has NO set_read_timeout/SO_RCVTIMEO → ENOPROTOOPT; use non-blocking+poll). Probe
  `repros/wasi-udp-probe` (wasi:cli/command warpkg war.probe.udp): loopback +
  outbound STUN to stun.l.google.com → srflx address. GREEN on desktop wasmtime AND
  on-device via `wart-host --run-once war.probe.udp` (Pixel 2 XL WiFi). Host
  candidates need no iface-enum either (bind+connect a UDP sock to a public IP,
  read local_addr). So sans-IO rtc + std UdpSocket = complete guest transport.
  Remaining call-engine work: Opus (libopus→wasip2) + wire engine↔audio + host
  crypto iface (perf). Committed in repros/wasi-udp-probe.
- **OPUS CODEC: DONE/de-risked (2026-06-02, device-verified, ~40x real-time).**
  NO libopus/wasi-sdk needed — `restsend/opus-rs` is PURE-RUST Opus (RFC 6716,
  ported from libopus 1.6, zero runtime deps). Builds for wasm32-wasip2 trivially;
  f32 encode/decode API matches our PCM-f32 pipeline (mic capture + AAudio).
  Round-trip (48k mono 20ms): 960 f32 → ~160B (~64kbps) → 960 samples, desktop +
  on-device (`wart-host --run-once war.probe.opus`). PERF on Pixel 2 XL (scalar,
  no SIMD): encode 0.384ms + decode 0.117ms = 0.501ms per 20ms frame = ~40x
  real-time (~2.5% budget) → comfortably real-time; +simd128 optional. Gotcha:
  Application::Voip (SILK) attenuates a pure TONE ~13dB (speech-tuned, expected;
  real speech fine) — test functionality not tone fidelity. repros/opus-wasip2
  (aed03f17). +simd128 MEASURED no-op on opus-rs (0.501→0.495ms, opus-rs has no
  hand-SIMD; LLVM can't auto-vec SILK's data-dependent loops). Codec-choice note:
  evaluated `wavey-ai/libopus-rs` (also pure-Rust, `forbid(unsafe_code)`, nice
  wasm bench tooling, and INDEPENDENTLY found wasm-simd128 SLOWER than scalar —
  corroborates us) but it's CELT-ONLY (no SILK yet) → can't do speech/VoIP →
  unusable for voice calls today. restsend/opus-rs is full SILK+CELT (Voip mode
  device-verified) → the choice. Watch wavey-ai: if SILK lands, forbid-unsafe +
  tooling make it an attractive swap. **CALL-ENGINE TRANSPORT+CODEC NOW ALL DE-RISKED; remaining = pure
  ASSEMBLY (wire rtc ↔ UDP ↔ Opus ↔ audio in a guest) + optional host crypto.**
