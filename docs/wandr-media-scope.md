# wandr:media — RETIRED (2026-07-20). Was: scope decision (2026-06-12)

> **RETIRED — `wandr:media` will not be built.** Both jobs it reserved have
> been taken by shipped packages, and its stated justification was dissolved by
> its own prerequisite. Kept as a decision record; do NOT start a `wandr:media`
> package from this file.
>
> | What it reserved | Where it went |
> |---|---|
> | **Transport** (play/pause/seek/rate, position/duration) | **`wasi:media-session`** — drafted 2026-06-14, two days after this file, and WIRED end-to-end (host impl + arbiter module + the audio player driving both directions). It argued the `wasi:` namespace for exactly this, reversing the "out of WASI scope" line below. |
> | **A/V sync** (host schedules video against the audio clock) | **task 117 M2** — and with the opposite answer: sync stays **guest-side**, because sync POLICY differs per player (live vs VOD, frame-drop vs audio-stretch). |
>
> **Why the justification dissolved.** The sketch below rests on one claim —
> *"the capability gap — neither side can see the other's clock from the
> guest"* (§Sketch). That gap was closed by **Prerequisite 1 of this very
> document**: `wasi:audio.playback.position` was promoted and shipped
> 2026-06-14 (task 108 M1, device-verified). The guest can now read the master
> clock directly, so a host-side scheduler is no longer the only way to sync —
> and per M2 it is no longer the preferred way.
>
> **What survives from this file:** the "no monolithic `wasi:media`" ruling
> (below) is still in force and still correct — the WASI-facing layer stays
> orthogonal pieces. Only the `wandr:media` composition package is retired.

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

## Prerequisites (named lanes already recorded) — all three resolved

1. `wasi:audio.playback.position` (frames played — the master clock):
   R2 additive method on the draft.
   → ✅ **SHIPPED 2026-06-14** (task 108 M1, device-verified). This is what
   dissolved the capability gap above, and therefore this package.
2. An audio-decode decision (guest-side codecs vs host passthrough).
   → ✅ **DECIDED: guest-side by default** (Symphonia), because audio decode is
   cheap (~1–3 % CPU) and it keeps codec licensing with the app author.
   `wasi:audio-codec` is the OPTIONAL offload lane, still unwired.
3. A player-class consumer to drive the union (R1 discipline — design
   from a shipped need, not speculation; the call engine is NOT it:
   calls need no transport/seek and sync at the RTP layer).
   → ✅ **ARRIVED** — `wandr.audio.player` (shipped) for the audio half, and
   task 117 M2's Jellyfin/YouTube clients for A/V. R1 held: the consumer
   arrived first, and it showed the union should be **guest-composed**, not a
   host `resource player`.
