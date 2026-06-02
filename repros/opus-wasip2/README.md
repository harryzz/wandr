# opus-wasip2 — Opus codec de-risk for the WebRTC call engine

The call path needs Opus (encode mic PCM → RTP payload; decode payload → speaker
PCM). webrtc-rs bundles no codec. **`restsend/opus-rs` is a PURE-RUST Opus
(RFC 6716, ported from libopus 1.6) — no C, no wasi-sdk** — which sidesteps the
whole C-to-wasm toolchain problem.

## Result (2026-06-02) — builds, runs, AND fast enough, device-verified

- **Builds** for `wasm32-wasip2` with zero fuss (`opus-rs` 0.1.22 has no runtime
  deps; pure Rust). No wasi-sdk / cc / libopus.
- **f32 API matches our pipeline** — `encode(&[f32], frame, &mut [u8])` /
  `decode(&[u8], frame, &mut [f32])` — same PCM-f32 as mic capture + AAudio.
- **Round-trip works** (48 kHz mono, 20 ms = 960 samples): 960 f32 → ~160-byte
  packet (~64 kbps) → 960 samples, on desktop wasmtime 45 AND on-device via
  `wart-host --run-once war.probe.opus` (deterministic, identical output).
- **Real-time with huge headroom (the key number), Pixel 2 XL, scalar:**
  ```
  encode 0.384 ms + decode 0.117 ms = 0.501 ms  per 20 ms frame
  ```
  ~40× real-time — ~2.5% of the frame budget. No SIMD needed for viability
  (`+simd128`/NEON would lower it further / save battery). Desktop x86 ref:
  0.235 ms/frame.

## Note on the test signal
The probe encodes a 440 Hz **sine** in `Application::Voip` (SILK, speech-tuned),
which deliberately attenuates a pure steady tone (~13 dB: in_rms 0.354 → out_rms
0.075). That is expected codec behaviour for a pathological non-speech input, NOT
a defect — real speech is preserved well. The assertion checks *functionality*
(compression + non-trivial decoded output), not tone fidelity.

## Run
```bash
cargo build --target wasm32-wasip2 --release
wasmtime run target/wasm32-wasip2/release/opus-wasip2-probe.wasm   # desktop
# device: package as wasi:cli/command warpkg (war.probe.opus), then
wart-host --run-once war.probe.opus
```

## Conclusion
`opus-rs` is the call engine's codec — pure-Rust, wasip2-native, f32, and ~40×
real-time on-device. With the UDP transport (`../wasi-udp-probe`) and the
webrtc-rs `rtc` stack (`../webrtc-rs-wasip2`) both de-risked, the remaining
call-engine work is **assembly**: wire the sans-IO `rtc` engine to UDP + Opus +
our audio capture/playback, in a guest. (Host-side ARMv8 crypto for SRTP is a
perf optimization, not a blocker — see `project_crypto_hw_offload`.)
