---
name: project-signal-resume-point
description: Signal client — current shipped state + where to resume (paused 2026-05-31)
metadata: 
  node_type: memory
  type: project
  originSessionId: 81538868-ab9d-48a4-8de3-a56739b11c3e
---

Signal work **paused 2026-05-31** at the user's request. Where it stands + resume
points. Code lives at `apps/user/war.signal/` (`engine/` + `ui/`); build/deploy is
`apps/user/war.signal/build.sh [--deploy]`. See [[project_signal_app_location]],
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

## NOT done / next (in rough order of tractability)
- **Send delivery/read receipts for messages WE receive** — we only consume incoming
  receipts to update our outgoing; we don't ack inbound, so the *other* side never
  sees "delivered" from us. Send `ReceiptMessage` on receive (+ read on open).
- **Attachments** (images, files, **voice notes**, **video clips**): the lib supports
  it (`AttachmentSpec`/`upload_attachment`/`get_attachment`, `attachment_cipher.rs`,
  CDN). Need host mic/camera capture + UI playback. Voice/video-as-attachment is the
  realistic media path.
- Persist unread across restart (currently in-memory); typing indicators; profile
  fetch beyond contacts-sync.
- **Real-time voice/video CALLS = OUT OF SCOPE for now**: libsignal-service-rs has
  only `CallMessage` *signaling*; real media needs **ringrtc** (separate heavy
  WebRTC native lib), not in our stack.

## Gotchas to remember
- Submodule `external/libsignal-service-rs` (branch `wart-wasi-transport`) pushes to
  origin = **codeberg.org/harryzz** fork, NEVER upstream whisperfish.
- Engine path-deps are 4-up (`../../../../external/...`) after the repros→apps move.
- Receipts match by the message's wire **ts**; `send()` and the outbox drain must use
  the SAME ts (fixed) or receipts won't correlate.
