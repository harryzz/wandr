# Task 70 — Per-conversation threading (send to a user/group, open a chat)

**Status:** DONE 2026-05-31 — device-verified on the Pixel 2 XL. Tap a contact or
group → its chat thread opens (filtered messages + composer + back); the **Chats**
tab lists conversations by name (contact / group title / "Note to Self") with a
last-message preview; `send(thread, text)` routes to the right 1:1 ACI or group;
incoming messages file into the correct thread. The engine also exposes
`account-id()` so the UI labels the self thread / self contact "Note to Self"
(in both the Chats list and the Contacts list).

**Group sender attribution:** each message stores `sender` = the actual sender's
ACI (independent of `thread`), so a group thread preserves who-said-what; the UI
renders the sender's name above every incoming bubble (`sender_label`). Your own
messages render as outgoing.

## Problem

The Signal client treats messaging as one global timeline:
- `send(text)` always targets **Note-to-Self** (`sender.send_message(&self_id, …)`).
- History is a single flat `/state/messages.jsonl` — no conversation key, so 1:1s
  and group messages are interleaved with no way to tell threads apart.
- Tapping a contact/group does nothing; there's no per-conversation view.

## What "fetch history from server" means (answered, won't build)

Signal is **store-and-forward**: the server holds an encrypted message only until
it's delivered once, then deletes it. A linked device receives **only messages
sent after it links** (plus the contacts/groups *metadata* sync — not bodies).
There is no protocol to backfill past history to a linked device. So threads fill
in **going forward** from live traffic; we persist what we capture.

## Approach

**Thread identity** = the conversation key on every message:
- 1:1 → the peer's ACI uuid (string). Incoming: the sender's ACI. Outgoing: the
  recipient's ACI. (Note-to-Self is just your own ACI.)
- Group → the group master key, base64 — same value as `group.id` from task 69.

**Engine**
- WIT: `message` gains `thread: string`; `send(text)` → `send(thread, text)`.
- `send(thread, text)`: route by thread — if it's a known group id →
  `send_message_to_group` with the member ServiceIds + a `GroupContextV2`
  (master key + revision); else 1:1 → `ServiceId::Aci(thread)`; empty/unparseable
  → Note-to-Self fallback. Outbox carries `(thread, text)`.
- Incoming attribution (`extract`): group → `DataMessage.group_v2.master_key`
  (b64); 1:1 → `content.metadata.sender.raw_uuid()`; sync-sent (my other devices)
  → outgoing + `Sent.destination_service_id` (or its group). Store sender as the
  raw uuid (UI resolves to a name).
- Group routing metadata (member ACIs + revision) kept in `Shared.group_routes`
  and persisted on `StoredGroup` (re-populated on connect anyway).
- `StoredMessage` gains `thread` (`#[serde(default)]` so old logs load).

**UI (signal-ui)**
- `current: Option<Thread>` — `None` = list view, `Some` = a chat window.
- List view tabs: **Chat** = conversation list (threads that have messages, last
  preview), **Contacts** = address book + groups. Tapping any row opens its thread.
- Thread view: back button + title header + messages filtered to the thread +
  composer that sends to `thread.id`.

## Verification (device)

Tap a contact → type → send → message appears in that thread (and arrives on the
phone). Tap a group → send → arrives in the group. Incoming 1:1 / group messages
land in the right thread. Back returns to the list; threads persist across restart.
