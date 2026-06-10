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
- **Signal engine** (`apps/user/wandr.signal/engine`): implements the provider via
  `wandr:crypto/aead-oneshot` (NOT the `aead` resource — see [[reference_wac_plug_imported_resource]]),
  calls `set_aead_provider` after `place`/`incoming`.

**Device-measured before/after** (Pixel 2 XL, `wandr.srtp.bench` `--run-once`, both
backends one run): AES-256-GCM, Signal V4 profile.
- audio 160 B: in-wasm 14 µs → host 4.8 µs = **3.0×**, ~1.4 → 0.47 ms CPU/s-of-call.
- video 1100 B: in-wasm 80 µs → host 9.8 µs = **8.2–8.7×**, ~24 → 2.8 ms CPU/s.
- throughput 11–14 MB/s (wasm SW) → 33 (audio) / 112–122 (video) MB/s host. (Raw
  probe-crypto host GCM = 221 MB/s; per-packet WIT boundary ≈ 4.6 µs floor.)
HW AES requires the `--cfg aes_armv8` + `--cfg polyval_armv8` rustflags in
`runtime/wandr-host/.cargo/config.toml` (NOT target-feature) — already set.

STATUS: library + engine wiring DONE + compiles; host (with `aead_oneshot::Host`) +
Signal redeployed, Signal **instantiates clean** under `--no-art` (was trapping on the
resource import). The actual SRTP-over-HW-AES runs only during a live call → needs a
user call to verify end-to-end (outgoing works; incoming media still blocked by
[[project_incoming_call_answerer_bug]]). NOT committed yet. Bench = `wandr.srtp.bench`.
Build Signal: `apps/user/wandr.signal/build.sh` (PROTOC=$HOME/tools/protoc/bin/protoc),
then `run-hybrid-stack.sh --wandr-only` so the zygote picks up the new host.
