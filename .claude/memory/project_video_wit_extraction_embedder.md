---
name: project_video_wit_extraction_embedder
description: "DONE (WIT design + WASI audit): stripped wasi:video-codec + wasi:camera surface-free, re-expressed wandr:video@0.1.0 as the embedder importing both (resolves clean = n&s proof), synced wandr:video-diag. Apps NOT rewired (deferred, by scope choice). Full design + per-app rewire in contracts/proposals/VIDEO-EMBEDDER-REWIRE.md."
metadata:
  node_type: memory
  type: project
  originSessionId: a14a1f7f-f5fb-44f9-a0e5-3879acecf911
  modified: 2026-08-06T14:12:09.778Z
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

## REWIRE PROGRESS (in flight)
- **✅ Desktop video.player slice VERIFIED (user-confirmed render).** Host serves the
  split `video-host` world (one bindgen aggregating wasi:video-codec/camera/eme +
  wandr:video present/capture-encode; second bindgen for diag with per-INTERFACE
  add_to_linker — world-level double-adds wasi:eme). Facades over the SAME
  video.rs/video_desktop.rs backend: codec decoder = decode-to-BUFFER; video-surface
  = embedder child surface (`present` = TakenFrame::present_to retarget on desktop;
  `attach` = set_surface_id for AUTO); call-encoder = fused VideoEncoder; eme = trapping
  stub. bbb-h265 plays 300/300, back-pressure preserved. Commits: host cc06e33,
  contracts 54a939b (wit/video/ relocation), parent 16aeb9ed.
- **GOTCHA**: contracts is vendored as a NESTED submodule in wandr-host — bump it
  (fetch+checkout the wandr-wit commit), don't rsync. Host bindgen path = the DIR
  `contracts/wit/video` (single-package + deps), not the file.
- **✅ media-engine + jellyfin**: rewired (decoder→surface; DASH rep-switch reuses the
  surface). jellyfin composes + run-verified on desktop (layered imports, no old fused).
  Bundled a pre-existing aspect-fit video_rect refactor (owner OK'd). Parent b0276627.
- **✅ Android CI fixed** (host c3eac74): video.rs `mod android` parity —
  video_surface_* over `media::` slots + TakenFrame accessors + set_surface_id.
  build-host-android.sh (NDK r27d) clean.
- **✅ Signal + video.test rewired** (parent 1748b690): Signal call.rs → call-encoder
  (outgoing) + wasi:video-codec decoder + video-surface.attach (incoming RTP AUTO);
  90kHz↔µs at the boundary. Full Signal app builds (6.5M wandrpkg). video.test (peer-free
  Android loopback) compiles. **ALL FOUR consumers on the layered world; none import the
  old fused wandr:video/{encoder,decoder}.**
- **✅ UNIFIED FLOW device-verified (host 7fc650e, parent 5f3c57f4)**. RULE: the guest
  path is OS/runtime-agnostic — ONE flow, both backends honor it:
  `open decoder → open surface → attach(&dec) → submit → next_decoded → present(f, at_ns)`.
  at_ns=0 = ASAP (Signal/RTP); computed = A/V-sync (players). NO host auto-present, NO
  two submit verbs. `attach` binds codec→surface (Android: stop→configure(window)→start,
  MediaCodec takes a surface only at configure + the split opens surface-free; desktop:
  set surface_id). `submit` HOLDS output (no slot requirement — part-1 buffer loopback
  has none). Android decoder-config is dimensionless → default 1280x720 at configure
  (MediaCodec rejects 1x1; resizes from the keyframe).
  - Guests changed: Signal now pulls+present(0) (was attach-then-never-pull → nothing
    ever presented, broken on BOTH backends); player/media-engine add attach; video.test
    part-1 drains, part-2 pull+present(0).
  - **Pixel 2 XL**: video.test part-1 (camera→HW-VP8→HW-decode) 60/61; part-2 (RTP
    pipeline, decode-to-surface) 204/205 @ 20.4fps — both PASS. Desktop video.player
    300/300 (no regression).
  - **DEPLOY (device)**: push host + app .wasm to an ISOLATED dir (/data/local/tmp/vt),
    push NDK libc++_shared.so + LD_LIBRARY_PATH, `--install` (host recompiles on cache-key
    mismatch, its OWN wasmtime — NO cwasm/CLI needed), `--run-once <app-id>`. Doesn't touch
    the running zygote.
- **🔲 REMAINING (visual/real-call)**: (1) eyeball on-screen video on the panel (part-2
  holds 30s; or --standalone); (2) a real Signal video call with a peer. Codec+attach+
  decode all PASS via the loopback proxy.

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
