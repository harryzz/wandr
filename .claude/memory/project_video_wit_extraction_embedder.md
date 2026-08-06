---
name: project_video_wit_extraction_embedder
description: "DONE (WIT design + WASI audit): stripped wasi:video-codec + wasi:camera surface-free, re-expressed wandr:video@0.1.0 as the embedder importing both (resolves clean = n&s proof), synced wandr:video-diag. Apps NOT rewired (deferred, by scope choice). Full design + per-app rewire in contracts/proposals/VIDEO-EMBEDDER-REWIRE.md."
metadata:
  node_type: memory
  type: project
  originSessionId: a14a1f7f-f5fb-44f9-a0e5-3879acecf911
  modified: 2026-08-06T07:59:01.592Z
---

# Video WIT extraction → wandr:video as embedder — DONE (WIT + audit); apps deferred

## STATUS (task 120, this session)
**WIT landed + WASI-audited; apps NOT rewired** (scope choice: WIT design + approval only;
re-expressing the wired contract would force rebuilding all consumers + zygote restart +
touch the production call path — a separate later pass). Full record + per-app rewire:
**`contracts/proposals/VIDEO-EMBEDDER-REWIRE.md`** (read it first next session).

Done:
- **`wasi:video-codec@0.0.1`** stripped surface-free (removed graphics-context dep,
  `connect(ctx)`, the separate `decoded-frame`/`present`/`discard`, `set-rotation`,
  `ready`, `surface-unavailable`, `open-accelerated`, the 90 kHz `encoded-frame`).
  `next-decoded` now returns the shared opaque `frame`. Mirrors `wasi:audio-codec`.
- **`wasi:camera@0.0.1`** stripped (removed graphics-context + `connect-preview`); kept
  enumeration; added privacy fix `camera-info.name: option<string>`.
- **`wandr:video@0.1.0`** re-expressed as the EMBEDDER: `interface present`
  (`video-surface`: open/attach/present(frame,at-ns)/set-rect/presented-rect/set-rotation)
  + `interface capture-encode` (`call-encoder`: fused camera→encoder + PiP) + `types`
  (z-layer/video-rect/surface-error/call-encoder-config). Worlds import
  `wasi:video-codec/{types,decoder,encoder}` + wandr interfaces.
- **`wandr:video-diag`** already held list-decoders/implementation/decoded-frames; synced
  its vendored `deps/video-codec` copy to the stripped shape.
- **Vendored deps** for the canonical embedder live in `contracts/wit/deps/{video-codec,
  camera,eme}/` (contracts/wit/ is multi-package so it can't resolve as a dir; verify
  video.wit in a temp single-package dir + deps).
- **n&s PROOF**: `wasm-tools component wit` resolves all four; the wasi packages `use`
  cleanly into `wandr:video` with NO forced change back into them.

## KEY DECISIONS (don't relitigate)
- **n&s = necessary for the app CLASS the runtime must serve, NOT "what the proof app
  calls today."** Proof apps only prove/test. So `read-rgba` (guest pixel readers),
  `encoded-chunk.decrypt` (CENC playback), and camera **enumeration** (`list-cameras`/
  `open-device` — desktop has no front/back, `facing` can't pick a device) all STAY,
  justified by app-class + a web-standard analogue. (User correction this session — this
  reversed the audit's "no call site → drop" nits.)
- **`frame` stays in `wasi:video-codec`** (parallel to audio-codec holding AudioData;
  WebCodecs defines VideoFrame). Future `wasi:screen-capture` imports it like camera does.
  (`wasi:video-frame` neutral-package extraction considered, declined — user's call.)
- **Camera switch = warm reopen** (drop session, `open` other facing / `open-device`);
  `set-active` = mute, not switch.
- **`present` is wandr, not codec** (WebCodecs has one VideoFrame, no present); host-fill
  decode-to-surface is a wandr stack decision (upstream wasi-gfx:surface is guest-pump).
- **WASI audit**: both `wasi:*` interfaces PASS all 6 criteria; removing `present`/`connect`
  is a net VIRTUALIZABILITY win. Doc nits folded in (queue-full comment, read-rgba as the
  copyTo escape hatch, frame display-dims, tail-removal ABI note, deferred-fields list).

## NEXT SESSION (the rewire, if/when approved)
Execute `VIDEO-EMBEDDER-REWIRE.md`: rewire Signal call.rs (encoder→call-encoder;
decoder+video-surface.attach; 90kHz→µs), video.player + media-engine (decoder+
video-surface.present; diag for list/implementation), each vendoring the layered world +
transitive wasi deps; split the host impl (wasi:video-codec + wasi:camera host impls + a
thin wandr:video embedder owning child surfaces + camera→encoder glue); rebuild all +
restart zygote; device-verify call A/V + playback + seek + subtitles.

## FILES
- `contracts/proposals/VIDEO-EMBEDDER-REWIRE.md` — the design + rewire (READ FIRST)
- `contracts/proposals/wasi-video-codec/wit/video-codec.wit` — stripped (template: `wasi-audio-codec`)
- `contracts/proposals/wasi-camera/wit/camera.wit` — stripped
- `contracts/wit/video.wit` — the embedder; deps in `contracts/wit/deps/`
- `contracts/proposals/wandr-video-diag/wit/video-diag.wit` — diagnostics
- Guests (design only): `apps/user/wandr.signal/engine/src/call.rs`,
  `apps/user/wandr.video.player`, `crates/wandr-media-engine`, `apps/user/wandr.jellyfin`

Related: [[reference_wasi_gfx_ecosystem_relation]] · [[feedback_wit_changes_need_approval]]
· [[feedback_shared_wit_rebuild_all_consumers]] · [[project_wandr_video_host]]
· [[project_wandr_call_video_track]] · [[project_task115_wasip3_async]].
