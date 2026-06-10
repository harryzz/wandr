---
name: project_wandr_crypto_srtp_offload
description: SRTP AEAD offloaded from in-wasm software AES to host HW AES via wandr:crypto — device-measured 3x audio / 8.5x video; Signal call engine cut over
metadata: 
  node_type: memory
  type: project
  originSessionId: a2edab94-9d77-4289-807e-6fabf67af25c
---

Task 93 follow-on (2026-06-10): moved the call engine's **SRTP per-packet AES-GCM**
off the in-wasm software `aes-gcm` (wasm can't reach ARMv8 AES) onto the host's
hardware AES via `wandr:crypto`. Architecture (keeps libs portable, WIT out of them):

- **rtc-srtp** (external/rtc submodule): new feature `external-aead` (off by default →
  crate unchanged/portable). Adds pure-Rust traits `AeadProvider`/`AeadCtx` (error type
  `()`, no deps) + `CipherExternalAead` reusing all RFC-7714 framing, delegating only
  the GCM block + `Context::new_with_aead(...)`.
- **wandr-call**: feature `host-aead = ["rtc-srtp/external-aead"]`, re-exports the traits,
  `MediaSession::new_with_aead`, `PeerSession::set_aead_provider`, `SignalCall::set_aead_provider`.
  WIT-agnostic — the guest injects the provider.
- **Signal engine** (`apps/user/wandr.signal/engine`): implements the provider via the
  `wandr:crypto/aead.aead-key` RESOURCE (host keys the GCM context once per SRTP
  session; no key bytes cross per packet) — switched back after proving the earlier
  "can't link resource through wac" belief false, see
  [[reference_missing_instance_error_stale_zygote]]. `aead-oneshot` stays as a
  convenience API (bench uses it; measured equal). Calls `set_aead_provider` after
  `place`/`incoming`. Device-verified instantiating clean via zygote.

**Device-measured before/after** (Pixel 2 XL, `wandr.srtp.bench` `--run-once`, both
backends one run): AES-256-GCM, Signal V4 profile.
- audio 160 B: in-wasm 14 µs → host 4.8 µs = **3.0×**, ~1.4 → 0.47 ms CPU/s-of-call.
- video 1100 B: in-wasm 80 µs → host 9.8 µs = **8.2–8.7×**, ~24 → 2.8 ms CPU/s.
- throughput 11–14 MB/s (wasm SW) → 33 (audio) / 112–122 (video) MB/s host. (Raw
  probe-crypto host GCM = 221 MB/s; per-packet WIT boundary ≈ 4.6 µs floor.)
HW AES requires the `--cfg aes_armv8` + `--cfg polyval_armv8` rustflags in
`runtime/wandr-host/.cargo/config.toml` (NOT target-feature) — already set.

STATUS: ✅ COMPLETE + LIVE-CALL-VERIFIED (2026-06-10): user placed a real Signal call
on the aead-key resource path — "audio works both ways" (every RTP packet sealed/
opened through host HW AES across the WIT boundary). The earlier "resource
implementation is missing" trap was a STALE ZYGOTE image, not a resource/wac problem.
Incoming-call media still blocked by the separate
[[project_incoming_call_answerer_bug]]. Commits 11c5efaa/266e6834/3c316895, pushed.
Bench = `wandr.srtp.bench`.
Build Signal: `apps/user/wandr.signal/build.sh` (PROTOC=$HOME/tools/protoc/bin/protoc),
then `run-hybrid-stack.sh --wandr-only` so the zygote picks up the new host.
