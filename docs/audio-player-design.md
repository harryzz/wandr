# Feature-rich audio player on wandr — design

> Written 2026-06-14 (task 108 scoping). Source-grounded: the shipped
> `wasi:audio@0.0.1` draft (`proposals/wasi-audio/wit/audio.wit`), its host
> impl (`runtime/wandr-host/src/wasi_audio_impl.rs`), `wandr:audio-focus`
> (`wit/audio-focus.wit`), the AudioFlinger-direct backend
> (`[[project_audioflinger_backend]]`), the SRTP HW-offload pattern
> (`[[project_wandr_crypto_srtp_offload]]`), and `wasi:video-decoder`
> (`proposals/wasi-video-decoder/`). W3C status verified 2026-06-14.

## Verdict / core architecture

A player is built as a **pure-Rust guest** over a **layered, capability-
negotiated** audio stack. The guiding principle — the same one the SRTP
offload already shipped — is **mechanism in the host, policy in the guest**:

1. **`wasi:audio` PCM device = the mandatory portable floor.** Guest decodes
   and DSPs in Rust → writes PCM. Always works, on any host, with zero HW
   dependency. HW is *always* optional optimization on top, never required.
2. **HW codec = optional capability** the guest *queries and opts into*
   per-stream (`wasi:audio-codec`, WebCodecs-shaped). Absent or refused →
   the guest decodes it itself. "Use the HW if it's there; write my own when
   it isn't" is one `match` on the open result.
3. **HW effects/DSP = optional capability**, same shape (`wandr:audio-effects`,
   Android-effect-shaped). Attach host EQ/etc. to the stream, or do biquad in
   Rust. Guest picks.

Everything else a player needs — demux, seek, tags, album art, gapless,
crossfade, ReplayGain, spectrum/waveform, playlists, network streaming — is
**guest-side and needs no new WIT**. The contract additions are deliberately
minimal (§6).

## Why this shape (not host-default decode)

Audio decode is **cheap** (~1–3 % CPU for stereo MP3/AAC/FLAC on a Pixel 2
XL). The pressure that forced `wasi:video-decoder` host-side — realtime video
decode is *impossible* in wasm — simply isn't present for audio. So a pure-
Rust decoder in the guest is fully adequate, and guest-default wins on every
axis we care about: it keeps `wasi:audio` a pure PCM contract (portable to any
host), keeps codec **licensing** with the app author (MP3 patents expired
2017; AAC is still partially encumbered), adds no host attack surface, and
gives seek/tags/duration for free (the demuxer already has them). HW offload
is then a pure *optimization* the guest reaches for when it pays — which is
exactly the selectable model below.

## Capability negotiation — the SRTP pattern generalized

This is not new architecture. `[[project_wandr_crypto_srtp_offload]]` already
ships "guest-selectable HW-or-custom": SRTP uses an `external-aead` trait that
routes to host HW-AES **or** the guest's own crypto, and the guest decides.
Generalize that trait-injection idea to codecs and DSP and you get this stack.
It is also idiomatic WASI: capabilities are *granted* (no ambient authority),
absence is normal, so **query-then-fall-back** is the native idiom. WebCodecs
itself has `isConfigSupported()` for exactly this dance; we make the HW-vs-
custom choice *explicit to the guest* (the WASI-appropriate move) rather than
hiding it behind host policy.

### Layer 0 — `wasi:audio` PCM device (floor, mandatory)

The shipped draft, unchanged in spirit: `playback` (write f32, buffered-frames,
start/pause) + `capture`, routing *intent* via `stream-class`. A minimal host
implements only this and every player still runs. One addition (§6): promote
the already-named `playback.position` clock.

### Layer 1 — `wasi:audio-codec` (optional HW codec; mirrors WebCodecs)

Mirrors the **W3C WebCodecs `AudioDecoder`/`AudioEncoder`** shape and parallels
the in-tree `wasi:video-decoder`. The guest probes, then chooses a lane:

```
probe(codec) -> codec-caps          // "do you have HW AAC?"  (hw / sw / none)
decoder.open(config) -> result<..., codec-error>   // codec-error reuses the
                                                   // video enum: unsupported-
                                                   // codec / no-hw-codec / ...
```

Two output **topologies** (the real decision — see §3):
- **Transcode**: HW-decode compressed → return PCM to the guest (`AudioData`-
  style). Composable: guest can EQ / mix / visualize, then write to Layer 0.
- **Tunnel**: HW-decode → straight to the sink (connect to a `playback`),
  PCM never returns — Android "offloaded" playback; the CPU can sleep.

Encoder is symmetric: HW AAC record (MediaCodec) vs guest Opus/AAC encode.

### Layer 2 — `wandr:audio-effects` (optional HW DSP)

The host advertises its hardware/framework effect set — on Android the
standard `AudioEffect` chain attachable to a track's session (Equalizer,
BassBoost, Virtualizer, LoudnessEnhancer, PresetReverb, plus the capture-side
AEC/NS/AGC). The guest either **attaches** an effect to its stream (HW path)
or does its own biquad/`fundsp` EQ in Rust (custom path). Params are exposed
with **portable meaning** (EQ band gains in dB), never vendor-specific tuning.

## The transcode-vs-tunnel decision: a good player exposes both

These two are mutually exclusive **by physics, not by our design**: tunnel
gives the PCM to the hardware, so the guest can't see it. Therefore:

- **Foreground, interactive** (visualizer, custom EQ/crossfade, A/V sync) →
  **transcode** (HW decode, PCM back) *or* guest decode. The guest owns the
  PCM, so it can FFT/mix/EQ it.
- **Background, screen-off** (hours of music/podcast) → **tunnel** (HW decode
  + HW effects straight to the sink). Max battery; the CPU sleeps. The guest
  gives up its own visualization/DSP for that stretch — which it isn't using
  while the screen is off anyway.

So a good player **exposes both and switches per-situation** — transcode/guest
while foregrounded with a visible waveform, tunnel when it goes to the
background or the screen blanks. Visualization + custom-DSP and tunnel-offload
can't coexist, and that's fine: nothing needs both at once.

## Host portability — the contract is OS-agnostic; AudioFlinger is just today's backend

The capability-negotiation model is *why* this is portable, not Android-bound.
The WIT says nothing about Android: Layer 0 is "PCM in/out + a routing-intent
class," Layer 1 is "decode these `EncodedAudioChunk`s if you can," Layer 2 is
"attach an effect with these portable params if you have one." A host on
**non-Android Linux (PipeWire/ALSA/PulseAudio), macOS (CoreAudio), or Windows
(WASAPI)** — any OS running wasmtime — implements Layer 0 against its own audio
API and simply **advertises no HW codec / no HW effects**; the guest's
fallback path (Rust decode + Rust DSP) then carries the whole player with zero
code change. The x86_64 desktop dev loop (`[[project_desktop_dev_loop]]`)
already proves wandr-host runs off-device, so a portable Layer-0 backend there
is the natural first non-Android target. **Not a requirement now** — recorded
so the contracts don't accrete Android-isms that would block it later (rule 4
below). AudioFlinger-direct is the *current* backend, not the contract.

## W3C standards alignment

There is no single "W3C Audio" to clone (the way wasi-webgpu clones WebGPU);
it's a family split by layer (verified 2026-06-14):

| Our piece | W3C/web standard to align to | Status |
|---|---|---|
| `wasi:audio` PCM device | **none** — the web hides the raw device inside `AudioContext` (closest: `AudioWorkletProcessor.process()` + `getUserMedia`). WASI-charter audio slot, not a W3C mirror | n/a |
| `wasi:audio-codec` (HW decode/encode) | **WebCodecs `AudioDecoder`/`AudioEncoder`** (`EncodedAudioChunk` ↔ `AudioData`, `isConfigSupported`) | W3C WD (Jun 2026) |
| DSP/EQ/spectrum (guest-side) | **Web Audio API** node graph (BiquadFilter/Analyser/Panner/Worklet) — we do it in Rust, don't mirror the graph | W3C Rec 1.0 / WD 1.1 |
| `wandr:media-session` (transport/now-playing) | **W3C Media Session API** (`metadata` + `setActionHandler` + `setPositionState`) — direct template | W3C spec, widely shipped |
| Network streaming (HLS/DASH) | Media Source Extensions — guest-side demux concern | W3C Rec |

So `wandr:media-session` and the optional `wasi:audio-codec` lane each get a
real W3C spec to track; the core PCM contract correctly has none.

## Feature map — where each piece lives

| Feature | Home | New WIT? |
|---|---|---|
| Decode mp3/aac/flac/alac/vorbis/wav | guest (Symphonia) | no |
| Decode Opus | guest (`external/opus-rs`, in tree) | no |
| Demux mp4/mkv/ogg/webm/caf | guest (Symphonia) | no |
| HW decode/encode (when present + chosen) | host via `wasi:audio-codec` | **yes (L1)** |
| Tags/metadata, duration | guest (Symphonia / `lofty`) | no |
| Album art | guest reads bytes → host `graphics.decode-image` | no |
| Seek | guest (Symphonia seek → write PCM) | no |
| Resample → device rate | guest (`rubato`) | no |
| Gapless / crossfade / ReplayGain | guest DSP pre-write | no |
| EQ / effects (custom) | guest (biquad / `fundsp`) | no |
| EQ / effects (HW, when chosen) | host via `wandr:audio-effects` | **yes (L2)** |
| Spectrum / waveform viz | guest (`rustfft`/`realfft`) → `wasi:canvas` | no |
| Network streaming | guest via `wasi:http` / the `wasi:tls` shim | no |
| **Transport clock (position)** | `wasi:audio` | **promote (L0)** |
| **Lockscreen/notification transport, now-playing, media buttons** | new `wandr:media-session` (arbiter-owned) | **yes** |
| Focus / ducking / route / volume / mute | `wandr:audio-focus` (shipped) | no |
| Background wakelock | arbiter (`[[project_arbiter_audio]]`) | no |

## Contract additions (minimal, each a named lane)

1. **`playback.position() -> u64` (frames played)** in `wasi:audio` — promote
   the already-named R2 deferral. The master clock for the progress bar,
   accurate-seek confirmation, and A/V sync (pairs with the video decoder's
   90 kHz ts). The player is its promoting consumer. Optionally add `drain()`
   (play-out-then-stop, vs `pause` which retains) to end a track clicklessly.

2. **`wasi:audio-codec@0.0.1`** (new, optional) — WebCodecs-shaped HW
   decode/encode with `probe` + transcode/tunnel output. Reuses the
   `wasi:video-decoder` error vocabulary (`unsupported-codec`/`no-hw-codec`)
   so guest fallback is one `match`. Separate package so a minimal host ships
   only Layer 0.

3. **`wandr:audio-effects@0.1.0`** (new, optional) — attach host effects to a
   stream, portable params. Separate package, `wandr:` namespace (no cross-
   platform standard for *host* effects yet; Web Audio is guest-side).

4. **`wandr:media-session@0.1.0`** (new) — arbiter-owned, like
   `wandr:audio-focus`/`wandr:alarm`/`wandr:notify`. Guest publishes
   now-playing metadata + state + position; the arbiter renders the
   lockscreen/notification transport and routes **headset/BT media-button**
   events (play/pause/next/prev/seek) to the guest's handler. Tracks the W3C
   Media Session API shape. The "platform owns transport UI" red line — the
   audio analog of the canvas-windowing split. The largest genuinely-missing
   piece and what makes a player feel native.

## Libraries (all pure-Rust, wasm32-wasip2, the shipped toolchain)

- **Symphonia** — demux + decode (FLAC/MP3/AAC/ALAC/Vorbis/WAV/AIFF + MP4/MKV/
  OGG/WebM) + tags. Royalty-free default-on; MP3/AAC behind feature flags.
- **opus** (`external/opus-rs`, in tree) — Opus.
- **rubato** — sample-rate conversion to the device rate.
- **lofty** — unified tag/art reading if Symphonia metadata is thin.
- **rustfft / realfft** — spectrum/waveform.
- **biquad / fundsp** — custom EQ/effects.
- Streaming: reuse the `wasi:tls` reqwest-shim (Signal) or wire `wasi:http`.

## Discipline (so flexibility doesn't rot the contract)

1. **The PCM floor is mandatory and sufficient** — HW is always optional.
2. **Every HW capability has a guest fallback** — portability never breaks;
   this is what makes it WASI, not an Android API.
3. **Capability query + typed "not available" errors** — reuse the
   `unsupported-codec`/`no-hw-codec` convention.
4. **No non-portable knobs** — portable param meaning only (dB, Hz), never
   vendor tuning; no Android-isms in the WIT (see §host-portability).

## Open questions / deferrals

- **Hi-res / bit-perfect** (96/192 kHz, 24-bit): the backend is f32 @ device-
  native (48 kHz). Hi-res output = R3 (format/rate expansion). Deferred.
- **>2 channels / spatial**: R3 (already a named wasi:audio deferral).
- **Bluetooth A2DP offload** (codec runs on the BT chip): out of scope; the
  `controls` route enum already reserves `bluetooth`.
- **Host decode-offload first build**: defer the *implementation* of L1/L2
  until the spike measures a battery win; the *contracts* can land first so
  the guest is written against the final shape.

## First step (spike → task 108)

A `apps/user/wandr.audio.player` guest: Symphonia decode of a local FLAC/MP3 →
`rubato` to 48 k → `wasi:audio.playback.write`, with a `wasi:canvas` UI (album
art via `decode-image`, a seekbar driven by the new `position`). Validates the
guest-decode floor, the one `position` addition, and art reuse — and tells us
whether HW offload is ever worth building. `wandr:media-session` is the
natural second milestone; `wasi:audio-codec`/`-effects` land only behind a
measured need. See `tasks/108-audio-player.md`.
