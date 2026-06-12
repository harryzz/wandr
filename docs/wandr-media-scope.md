# wandr:media — scope decision (2026-06-12, not designed yet)

User-set scoping: **no monolithic `wasi:media`** — players exist without
video, recorders without playback; the WASI-facing layer stays the
orthogonal pieces (`wasi:audio` PCM I/O, `wasi:video-decoder`/`-encoder`
codec↔surface, `wasi:canvas`). Media COMPOSITION is a wandr-local
concern: a future **`wandr:media`** package, out of WASI scope.

## What wandr:media would actually be (the R5-style justification)

The one thing the pieces cannot do alone is **synchronization +
transport**: a player needs an A/V clock (audio is the master — video
frames schedule against playback position) and a transport surface
(play/pause/seek/rate over a composed pipeline). Everything else IS the
pieces.

Sketch (defer WIT until a player-class consumer exists):

- `resource player`: owns one `wasi:audio.playback` + optionally one
  `wasi:video-decoder` connection; host schedules video frames against
  the audio clock (the capability gap — neither side can see the
  other's clock from the guest).
- transport: play/pause/seek/set-rate + position/duration queries.
- The guest (or a guest library) does demux + audio decode (opus/vorbis
  are pure-wasm-friendly); compressed-audio passthrough to host codecs
  is a named R3 lane on wasi:audio.

## Prerequisites (named lanes already recorded)

1. `wasi:audio.playback.position` (frames played — the master clock):
   R2 additive method on the draft.
2. An audio-decode decision (guest-side codecs vs host passthrough).
3. A player-class consumer to drive the union (R1 discipline — design
   from a shipped need, not speculation; the call engine is NOT it:
   calls need no transport/seek and sync at the RTP layer).
