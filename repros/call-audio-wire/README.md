# call-audio-wire — wandr-call's PCM ends wired to real mic/AAudio

The final call-engine integration: connect `wandr_call::MediaSession`'s PCM in/out
to the host `audio` WIT — **mic capture** in, **AAudio playback** out — and run it
through the real codec+crypto pipeline on real hardware.

## What it does

A `wasi:cli/command` guest (`wandr.probe.callaudio`) that:
1. **Captures** ~3 s of the device mic via `audio.open-capture` / `read-pcm-f32`.
2. **Runs each 20 ms frame through `MediaSession`** — Opus encode → RTP → SRTP →
   SRTP⁻¹ → RTP → Opus decode (loopback keys, the same media plane a live call
   uses), buffering the result.
3. **Plays the buffer back** via `audio.create-track` / `write-pcm-f32`.

So you **speak, then hear yourself** back through the exact pipeline a WebRTC call
runs. It's **sequential** (record fully, then play) because this device can't hold
input + output MMAP endpoints at once — in a real two-device call that limit never
bites, since each device only captures (→ peer) or plays (← peer).

```
mic ─read-pcm-f32→ PCM ─MediaSession.send→ Opus/RTP/SRTP ─┐ (loopback keys)
speaker ←write-pcm-f32─ PCM ←MediaSession.recv─ SRTP/RTP/Opus┘
```

## Build + run (device)

```bash
cargo build --target wasm32-wasip2 --release        # already a wasi:cli/command
cp target/wasm32-wasip2/release/call-audio-wire.wasm components/probe.wasm
# wandrpkg = this dir's package.toml + components/probe.wasm; push it, then:
wandr-host --install <wandrpkg>                         # app_id wandr.probe.callaudio
wandr-host --run-once wandr.probe.callaudio             # speak during "capturing…"
```

## Result (2026-06-02) — audibly device-verified ✅

```
[callaudio] capturing ~3 s of mic → wandr-call (Opus + SRTP loopback) — speak now…
[callaudio] 150 frames through wandr-call (144000 samples out); mic_rms=… out_rms=…
[callaudio] playing 1.5 s reference tone + your captured mic ×30.0 (peak …)…
[callaudio] DONE — mic → wandr-call (Opus + SRTP) → AAudio, end-to-end on real hardware
```

**Confirmed on a Pixel 2 XL: the reference tone and the captured voice both play
out the speaker** — the mic feeds wandr-call's encoder and wandr-call's decoder
feeds the speaker, the full audio plane of a call, on real hardware. (The mic
plays back amplified because this device's mic sits near the noise floor — a gain
question, not a wiring one.)

## Three device gotchas (all handled here)

1. **Stereo-only MMAP output.** The Pixel 2 XL's MMAP *output* endpoint rejects a
   mono track (`openMmapStream → -38`, then `-889`). Play a **stereo** track and
   interleave the mono pipeline output L = R. (Capture is fine in mono.)
2. **No simultaneous input+output MMAP.** Hence record-then-play, not a live
   monitor loop. Not a call limitation (one direction per device).
3. **Write-then-start (DMA prime).** The output is a shared-memory ring the HAL
   DMA-pulls; if you `start()` it empty it never begins pulling (the ring stays
   full, writes return 0, ~32 s of grind for 4.5 s of audio, silence). **Prime the
   ring with PCM first, *then* `start`**, then stream the rest — playback runs at
   real-time and is audible.

## What's NOT here

This is the *audio↔engine* wiring with loopback SRTP keys. A live call adds the
real peer: DTLS-derived keys + ICE + UDP + signaling — all already proven
(`../call-udp-loopback`, `../call-browser`, `../call-interop`). Composing this
audio wiring with a `PeerSession` over the network = a complete call.
