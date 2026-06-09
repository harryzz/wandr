---
name: project-signal-resume-point
description: Signal client — current shipped state + where to resume (paused 2026-05-31)
metadata: 
  node_type: memory
  type: project
  originSessionId: 81538868-ab9d-48a4-8de3-a56739b11c3e
---

Signal work **paused 2026-05-31** at the user's request. Where it stands + resume
points. Code lives at `apps/user/wandr.signal/` (`engine/` + `ui/`); build/deploy is
`apps/user/wandr.signal/build.sh [--deploy]`. See [[project_signal_app_location]],
[[project_signal_client_architecture]], [[project_wart_step_executor]].

## Shipped + device-verified (Pixel 2 XL)
- Link as secondary device; resume across restarts (account/store in `/state`).
- Contacts sync (names/phones/avatars) + Groups v2 (titles/members/avatars) via the
  storage service; master key derived from the AEP (`derive_svr_key`) at link.
- Per-conversation threading: tap a contact/group → chat; `send(thread,text)` routes
  1:1 (ServiceId::Aci) or group (`send_message_to_group` + GroupContextV2); incoming
  filed by thread; group msgs keep per-sender attribution. "Note to Self" labeled.
- Conversation polish: local timestamps (offset derived from host `status::clock-text`
  vs UTC), date dividers (Today/Yesterday/date), delivery/read receipts
  (✓/✓✓/✓✓-blue, matched by wire ts → `statuses.json`), unread badges (in-memory).
- Emoji: host fix — `canvas_impl.rs` shapes blobs via SkShaper w/ FontMgr fallback
  (dioxus-canvas path lacked it; Compose's Paragraph path always had it). Host-wide.
- Chat-render polish (2026-05-31, commits be923f7f/ed9cd4f7): long message lines
  WRAP (bubble `max-width:78%` + `white-space:normal`); conversation opens scrolled
  to the newest msg + sticks to bottom (`data-stick-key={thread.id}`); typed spaces
  render + caret advances. All three are dioxus-canvas renderer capabilities — see
  [[reference_dioxus_taffy_rust_ui]] (wrapping is opt-in; min-content + Skia
  trailing-whitespace gotchas).
- Emoji reactions (2026-05-31, commit 06d92f70): incoming `DataMessage.reaction`
  (+ sync-sent) matched to its target by wire SEND ts → `message.reactions` string
  + `reaction-changed` event; persisted reactions.json. Group = per-reactor map so
  multiple reactions show, repeats counted ("❤️3 👍"). REQUIRED changing incoming
  msg `ts` from receive-time to `dm.timestamp` (send time) so reactions match —
  so only msgs received post-this-build can be reaction-matched (old history rows
  stored receive-time ts). Reactions on OUR outgoing msgs always matched (ts=wire).

- Send receipts (2026-05-31, commit dff5953a): we now ACK inbound — delivery receipt
  on receive (to the sender; group → each member), read receipt via new `mark-read`
  WIT call the UI fires on open_thread + when a msg lands in the open thread. Drained
  in the send tick; `read_acked` set prevents re-acks. So peers see ✓/✓✓ from us.
- Inbound image attachments (2026-05-31, commit af960433): engine downloads
  (`push.get_attachment`) + decrypts (`decrypt_in_place`, 64-byte key, truncate to
  size) `image/*` attachments → `attachment` record on `message`; bytes persisted
  under `/state/att/`. UI renders aspect-fit `img`; list shows "📷 Photo". RECEIVE-ONLY,
  images only (video/files/voice download-skipped; same path extends). data: URI is
  `Rc`-shared so per-frame snapshot() clone is cheap.
- Adaptive idle frame pacing (2026-05-31, commit e25e83fd): the engine-poll loop cost
  a steady ~14% CPU (fixed 8fps repaint to service the live socket; same in convo &
  list → per-frame cost, not render complexity). Now pre_frame ramps min_frame_delay
  120→250→500ms after idle (IDLE_FRAMES counter, reset on engine activity) → ~4-6%
  idle, receive still <0.5s, input-driven frames keep interactivity. **The whole
  screen repaints every render_frame even when not dirty — frame rate is the lever.**

## NOT done / next (in rough order of tractability)
- **Send image attachments** (we only RECEIVE them) — needs `upload_attachment` +
  attaching an `AttachmentPointer` to the outgoing DataMessage + a UI image picker.
- **Other attachment types on receive**: files, **voice notes**, **video clips** —
  same download/decrypt path, just not filtered to `image/*`; need UI playback (+ host
  mic/camera for *sending* voice/video). Voice/video-as-attachment is the realistic
  media path.
- **Profile pictures** (the higher-res photo a user sets, vs contacts-sync avatars):
  separate **Profile API** — contact's profileKey (from contacts-sync) → GET
  /v1/profile/{aci} → decrypt avatar from CDN. NOT attachments.
- Persist unread across restart (currently in-memory); typing indicators.
- **Real-time voice/video CALLS = OUT OF SCOPE for now**: libsignal-service-rs has
  only `CallMessage` *signaling*; real media needs **ringrtc** (separate heavy
  WebRTC native lib), not in our stack.

## Gotchas to remember
- Submodule `external/libsignal-service-rs` (branch `wart-wasi-transport`) pushes to
  origin = **codeberg.org/harryzz** fork, NEVER upstream whisperfish.
- Engine path-deps are 4-up (`../../../../external/...`) after the repros→apps move.
- Receipts match by the message's wire **ts**; `send()` and the outbox drain must use
  the SAME ts (fixed) or receipts won't correlate.
