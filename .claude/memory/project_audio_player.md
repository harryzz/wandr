---
name: project_audio_player
description: "Task 108 audio player design (scoped 2026-06-14, not built): layered capability-negotiated audio stack — wasi:audio PCM = portable floor (guest decodes in Rust); HW codec (wasi:audio-codec/WebCodecs-shaped) + HW effects (wasi:audio-effects) = optional, guest queries+opts-in+falls-back; transcode-vs-tunnel; promote playback.position; new wasi:media-session. Design = docs/audio-player-design.md, task = tasks/108-audio-player.md"
metadata: 
  node_type: memory
  type: project
  originSessionId: 00de4e50-ab0d-4032-8361-7d93d41cf043
---

**Task 108 — feature-rich audio player.** Scoped 2026-06-14, NOT built.
Design: `docs/audio-player-design.md`; task: `tasks/108-audio-player.md`.

**Core architecture = layered, capability-negotiated (mechanism in host,
policy in guest) — the SRTP HW-offload pattern [[project_wandr_crypto_srtp_offload]]
generalized to codecs + DSP:**
- **Layer 0 — `wasi:audio` PCM device = mandatory portable floor.** Guest
  decodes + DSPs in Rust → PCM write. Works on ANY host, zero HW dependency.
- **Layer 1 — `wasi:audio-codec` (optional HW decode/encode, WebCodecs-shaped,
  parallels `wasi:video-decoder`).** Guest `probe`s, opts in per stream;
  absent/refused → guest decodes itself ("use HW if present, write own when
  absent" = one `match`, reusing video-decoder's `unsupported-codec`/
  `no-hw-codec` errors).
- **Layer 2 — `wasi:audio-effects` (optional HW DSP).** Attach Android
  AudioEffect (EQ/BassBoost/Virtualizer/Loudness/Reverb) to the stream, OR
  Rust biquad/fundsp. Portable params only (dB/Hz).

**Why guest-default decode:** audio decode is CHEAP (~1-3% CPU) — the
realtime-impossibility pressure that forced `wasi:video-decoder` host-side is
absent. Guest-default keeps `wasi:audio` a pure PCM contract (portable),
licensing guest-side (MP3 patents expired 2017, AAC partial), seek/tags/
duration free from the demuxer.

**Transcode vs tunnel (HW decode topologies, the real decision):** transcode =
HW-decode → PCM back to guest (composable: EQ/viz/mix); tunnel = HW-decode →
straight to sink (CPU sleeps, max battery, guest LOSES the PCM). Mutually
exclusive by physics. A good player **exposes both, picks per-situation** —
transcode/guest while foreground w/ a visualizer, tunnel when backgrounded/
screen-off. (= the answer to "offloaded/tunneled".)

**Host portability (note, not a requirement now):** the WIT is OS-agnostic —
AudioFlinger is just today's backend. A PipeWire/ALSA/CoreAudio/WASAPI host
(any OS running wasmtime; desktop dev loop [[project_desktop_dev_loop]] already
runs host off-device) implements Layer 0 + advertises no HW codec/effects →
guest fallback carries the whole player unchanged. Keep Android-isms OUT of
the contracts so this stays open.

**Gaps filled:** decode (Symphonia + `external/opus-rs`, guest) · transport
clock (**promote `playback.position`** — the ONE `wasi:audio` add, the master
clock for seekbar + A/V sync) · now-playing/lockscreen/headset-buttons (**new
`wasi:media-session@0.0.1`**, arbiter-owned like [[project_event_bus]]/alarm/
notify, tracks W3C Media Session API) · tags/art (Symphonia + `graphics.
decode-image`) · seek/gapless/crossfade/ReplayGain/EQ/spectrum/streaming (all
guest, NO new WIT).

**W3C alignment:** no single "W3C Audio" to clone (family split by layer,
verified 2026-06-14): PCM device has NO web analog (web hides it inside
AudioContext / AudioWorkletProcessor.process + getUserMedia) → cites WASI
charter slot; `wasi:audio-codec`←WebCodecs AudioDecoder/Encoder (W3C WD);
DSP/EQ/spectrum←Web Audio API (W3C Rec, but guest-side in Rust, not mirrored);
`wasi:media-session`←Media Session API. So media-session + the HW-codec lane
each get a real W3C spec to track; core PCM correctly has none.

**Libs (pure-Rust wasm32-wasip2):** Symphonia (demux+decode FLAC/MP3/AAC/ALAC/
Vorbis/WAV + MP4/MKV/OGG/WebM + tags) · opus (in tree) · rubato (resample) ·
lofty (tags/art) · rustfft/realfft (spectrum) · biquad/fundsp (EQ).

**Current state going in:** `wasi:audio@0.0.1` (PCM only, NOT wired) +
`wandr:audio-focus@0.1.0` (focus/route/volume/mute, shipped) over the
AudioFlinger-direct backend [[project_audioflinger_backend]] (f32 @ 48k
stereo). M1 spike = local FLAC/MP3 → player guest + `position`; M2 =
media-session; M4 = HW lanes ONLY behind a measured battery need.

**M1 DONE + USER-VERIFIED on device (2026-06-14):** `apps/user/wandr.audio.player`
— Symphonia 0.5.5 FLAC decode → wasi:audio, AUDIBLE on Pixel 2 XL (user-confirmed
clean). `playback.position` PROMOTED (proposal WIT + host `wasi_audio_impl.rs`
PlaybackRes.written counter; position = written − buffered), tracks wall ±40ms
(lead = counts our ring only, not the downstream AF mixer buffer; future: true
presentation ts via cblk mServer/AudioTimestamp). The guest STARTED as a
wasi:cli/command spike then became a **wasi:canvas UI reactor** (imports canvas/
audio, exports input-handlers frame+pointer): vinyl art placeholder, title/format
from tags, real WAVEFORM overview computed guest-side from PCM (showcases the
guest-DSP point), seekbar driven by `position`, play/pause + tap-seek via touch —
ALL user-confirmed working on device. 1.54 MB guest. Risk probe = repros/audio-
decode-probe. Launch: `wandr-arbiter launch wandr.audio.player` (needs host-108
for position; stack restarted --no-art on host-108). FINDINGS: (a) ✅ RESOLVED — `flush()`+`drain()`
ADDED to wasi:audio (WIT+host+guest), device-verified GAPLESS seek (2026-06-14):
flush=IAudioTrack.flush but MUST pause first (flush mid-play wedges the track →
no audio; the v1 bug), drain=IAudioTrack.stop; host keeps `position` continuous
by subtracting the dropped backlog from the `written` counter; player seek
rewired to flush→re-anchor(anchor_dev/anchor_track)→prime ring→resume (replaced
close+reopen + base_frame). audioclient exposes flush/stop/getTimestamp (the
last = future fix for the ~40ms position lead).
END→SEEK→PLAY BUG (2026-06-14, user-found, FIXED guest-side): when the track
played to end, the empty ring underran and AudioFlinger REMOVED the track
("prepareTracks_l BUFFER TIMEOUT: remove track ... due to underrun") — a removed
track can't be revived by flush/start (no audio). The guest-side fix = CLOSE the
track at end (pb=None) + reopen a FRESH track on replay/seek-after-end (reuses the
working open path; flush-seek stays for live mid-play). pause-at-end did NOT work
(AF removes within a buffer cycle ~21ms, before the guest's 33ms on_frame reacts —
the RACE). HOST-SIDE ROBUSTNESS = OPEN: ideally the host hides underrun-death so
no guest can break audio (mechanism-in-host). BUT the host deliberately keeps
MEDIA tracks "direct" (audio_impl comment: "media goes direct — media guests
buffer themselves"); the call jitter-PUMP solves underrun-death for CALL tracks
only. The naive host fixes each have a CATCH: (1) silence-padding the guest's
ring glitches the direct stream (silence interleaves with real audio); (2) the
call-pump model changes media's pending_frames/backpressure + position semantics;
(3) lazy re-create needs reliable dead-track detection (getTimestamp/underruns
only give it over a time-window). The sound host fix = move media playback onto
the host jitter-pump model (proven glitch-free for calls), preserving position +
backpressure — a real, careful change, best done when M4 extends the host audio
path. Guest close-at-end is a fine "good-citizen" workaround meanwhile.
(b) desktop winit window
unusable in the WSL sandbox (wayland connection reset) → verified via adb
screencap; (c) adb `input tap` doesn't work under --no-art (no system_server) —
touch routes via evdev/wandr-inputflinger, so interactive tests need real taps.
Album-art decode (graphics.decode-image) deferred — test FLAC has no art.
HOW (UI patterns, from the launcher): cdylib + wit_bindgen::generate!(world,
generate_all) + export!(Player); canvas via wembed::get_context()→get_current_
buffer()→draw_*→present; text via wlayout ParagraphBuilder; all geometry derived
from cv.width()/height() (no hardcoding). package.toml `world` field is
informational — UI vs command dispatch is by launch mode (arbiter launch =
instantiate+probe exports; --run-once = instantiate_command).

**MEDIA FAMILY SKETCHED 2026-06-14 (proposals/, NOT WIRED) — separate packages,
WASI modularity, each optional w/ guest fallback; W3C Media WG mirror:**
`wasi:audio-codec` (WebCodecs AudioDecoder/Encoder; probe=hw/sw/unsupported;
transcode=read vs tunnel=connect-to-playback; reuses video-decoder errors;
µs timestamps), `wasi:audio-effects` (Web Audio node subset: biquad/compressor/
gain/stereo-pan/reverb/delay, portable dB/Hz/s params, AnalyserNode+AudioWorklet
DELIBERATELY OUT = guest-side; wasi: ns), `wasi:media-session` (Media Session
API 1:1, arbiter-owned like audio-focus, host `session`+export `session-handler`),
`wasi:eme` (EME control plane; host owns CDM, guest shuttles opaque license
blobs via wasi:http = the SRTP-offload pattern; OWNS `decrypt-config`/
`encryption-scheme`/`subsample`). **DRM SCOPE DECIDED 2026-06-14 = ClearKey-ONLY**
(host AES-CTR, portable, self-hostable, covers self-served/Signal-style);
Widevine DEFERRED (TEE + Google-provisioned CDM, device-only) but reachable via
the key-system string with NO contract change.
`decrypt-config` + optional key-session added to BOTH audio-codec + video-decoder
chunks; SECURE-OUTPUT rule = robust DRM is sink/tunnel-only (no read; ClearKey
may read=testing). CROSS-PACKAGE DEPS NOW WIRED + wasm-tools-validated
(2026-06-14): audio-codec→{audio(playback),eme(decrypt-config,key-session)},
audio-effects→audio(playback), video-decoder→eme; each `use
<ns>:<pkg>/<iface>@<ver>.{..}` with the dep copied into that proposal's
wit/deps/<bare-name>/. WIT GOTCHAS hit: (1) `borrow<>` CANNOT live in a record →
key-session bindings are `open()` params, not config-record fields; (2) `stream`
is a reserved WIT keyword → can't be a param name (used `sink`); (3) nested
namespace wasi:media:x invalid (one colon). Headers still say "NOT WIRED" =
no HOST impl yet (contracts are complete drafts). MSE = NO package (proposals/wasi-media-source/NOTES.md: guest
orchestration over http/tls+codec+audio; the ONLY real host residue = DRM/EME,
which wasi:eme covers). Family table + ASCII stack diagram in the design doc.
Namespace logic (refined 2026-06-14): wasi:* = mirrors a W3C standard AND the
guest contract is portable (audio/codec/effects/eme/media-session — media-session
moved wandr:→wasi: because Media Session API is a shipped W3C std w/ universal
analogs MPRIS/SMTC/Now-Playing/Android; host rendering it via the arbiter is just
impl detail, red line held by contract SHAPE not namespace). wandr:* = NO W3C
standard / platform-idiosyncratic (audio-focus stays wandr:). Note: wasi:media:x
is INVALID WIT (one colon) + against WASI convention (wasi-gfx is flat) → flat
names + docs umbrella, no super-namespace. UI deferred until contracts settle.

**M2 PHASE 1 COMPLETE + USER-VERIFIED (2026-06-14) — wasi:media-session now-playing/lockscreen transport. User confirmed on device: real lockscreen taps drive the player, audio audible+clean, swipe-unlock works. LOCKSCREEN-ORIENTATION FIX (user-reported: lock appeared landscape when app was landscape): lock screen now ALWAYS portrait + no rotation (like launcher). Fix = wandr-arbiter-wm `effective_orient()` returns 0 when a `Role::Lockscreen` surface is present (the surface model IS the lock signal; no Store flag needed) + Lockscreen added to `fan_overlays` targets; keyguard module emits `Event::OrientationChanged` on lock/unlock so wm re-fans → locking from a landscape app snaps BOTH lock screen AND status bar to portrait, unlock restores the app's orient. Device-verified via `report-orientation <0..3>|nc -U …arbiter.sock` + lock + screencap. STALE-CONTROLS FIX (user: killed player but lockscreen controls stayed): ROOT CAUSE was host-side — `now_playing::get()`/`artwork()` didn't strip the arbiter's `"OK "` status-line prefix, so an empty reply (`"OK "`) failed `line.trim().is_empty()` (trim=="OK") and got parsed into `Some` with blank fields → drew the title-less bar+buttons. FIX = strip `line.strip_prefix("OK").map(str::trim)` then emptiness-check/parse the body (applies to ANY chrome/host reader of an arbiter `Reply` over the socket — the reply is always `OK <body>`/`ERR <body>`). + arbiter `MediaSessionModule::prune_dead(ctx)` backstop (retain only sessions whose pid `ctx.store.app_by_pid().is_some()`; recompute active) called on now-playing/artwork/action — drops stale sessions from rapid relaunches / missed SurfaceRemoved. Both device-verified: card shows when player live, fully gone when killed. SWIPE FIX: detect via peak Move-travel (max of release-delta and drag peak), not Down/Up delta — the Down/Up-delta way failed on touch once the now-playing card crowded the lower swipe area. Phase 2 (headset) deferred.**
Built like notify/alarm/audio-focus. NEW chrome read iface `wandr:chrome/now-playing`
(get/artwork/send-action) — wasi:media-session stays the PORTABLE guest contract,
the platform render-feed is wandr:chrome (namespace logic, like notify-feed). Host
`media_session_host_impl.rs` (session publish + chrome now-playing read, forwards to
arbiter over the line socket); new `wandr-arbiter-media-session` module (sessions map +
active-session = last set-metadata / set-state playing; SurfaceRemoved cleanup; routes
media-session-action -> active session host -> session-handler.on-action). keyguard
renders title+scrubber+prev/play/next + tap->send-action. player imports session
(publish)+exports session-handler(apply). Loop device-verified end-to-end via screencap +
`echo media-session-action <act>|nc -U /data/local/tmp/wandr-arbiter.sock` (CLI does NOT
forward module verbs — only the socket does; nc -U works on device).
TWO BUGS HIT+FIXED:
(1) EMPTY METADATA FIELD COLLAPSE: pct_encode("")="" -> empty token -> whitespace
split collapses the positional set-metadata wire -> arbiter rejects (needs 4 args) ->
title blank but transport showed (session existed via set-state). FIX = `~` sentinel for
empty fields (enc_field/dec_field both crates; `~` never emitted by pct_encode).
(2) ‼️ LOCKSCREEN-CONTROLLED PLAYER MUST BE A BACKGROUND-SERVICE: when the player is
backgrounded behind the lock it STOPS rendering (render skipped when Role::Background) ->
no pump() -> ring underruns -> AudioFlinger removes track -> "play dies after end/prev/next"
+ scrubber frozen + transport unresponsive. FIX = package.toml `background=true` + export
`wandr:background/background`; moved the audio engine into `engine_step()` called by
`bg-tick` (host pumps render-INDEPENDENTLY in EVERY role, clamp 16..IDLE_CAP) — on_frame
now ONLY renders. Verified: while covering=player(locked), set-position advances 0..N,
after-end prev+play recovers, no track-removal. (Generalizes: any guest controllable while
backgrounded = bg-service, like Signal.)
SWIPE-UNLOCK: user couldn't unlock; `git diff` CONFIRMS keyguard unlock logic
byte-identical (NOT my regression) — pre-existing gesture; the now-playing card crowds the
lower swipe area so a short swipe falls under the 12%-of-h threshold. Force-unlock:
`echo unlock|nc -U …arbiter.sock`. Phase 2 (headset/BT media-button via wandr_inputflinger
interceptKeyBeforeQueueing) DEFERRED — evdev device-risk (BT AVRCP may bypass HID; validate
with getevent first). Plan: ~/.claude/plans/cat-task-state-polymorphic-kernighan.md

Related: [[project_audioflinger_backend]], [[project_arbiter_audio]],
[[project_audio_routing_arbiter]], [[project_wandr_crypto_srtp_offload]],
[[reference_wasi_webgpu_gfx]], [[project_desktop_dev_loop]],
[[project_wandr_video_host]].
