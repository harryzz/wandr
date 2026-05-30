# Task 69 — Signal groups (Groups v2)

**Status:** OPEN (filed 2026-05-31, spun out of task 67 Phase 3 contacts).

## Problem

The contacts list (task 67 Phase 3) only has 1:1 users — **groups are missing**.
That's expected: the linked-device **contacts-sync** blob (`SyncMessage::Contacts`
→ `MessageReceiver::retrieve_contacts`) carries only individual `ContactDetails`,
no group data.

Groups need a separate mechanism:
- **Legacy group-sync** (`SyncMessage::Groups`, groups-v1) is deprecated (~2021);
  Signal no longer sends it and our fork doesn't expose a `retrieve_groups` for it.
  Dead end.
- **Groups v2** is the real path — but it's notably bigger than the 1:1 sync.

## Approach (Groups v2 via the storage service)

The modern source of "everything in your account" (contacts *and* groups) is the
**storage service**: an encrypted manifest the server holds, decrypted with the
account's **storage key** (derived from the master key). The fork already has the
building blocks:
- `src/storage_service.rs` — the manifest/record API.
- `src/master_key.rs` — master key / storage key derivation.
- `src/groups_v2/` (`model.rs`, `utils.rs`) — Groups v2 model + crypto.
- `src/sender.rs::send_message_to_group` — sending (the receive/list side is what
  we need first).

Sketch:
1. Get the **master key** — a linked device receives it during provisioning
   (`provisioning/mod.rs`) or via a keys-sync; from it derive the storage key.
2. Fetch + decrypt the **storage manifest** → records: account, contacts, and
   **group-v2 records** (each carrying a group **master key**).
3. For each group master key, derive the group's public params and fetch the
   **group state** from the groups server (`groups_v2`) → title, members
   (ACIs → resolve against the contact list from Phase 3), avatar, etc.
4. Persist to `/state` (e.g. `groups.json`, mirroring `contacts.json`), expose on
   the `wart:signal/chat` contract (a `group` record + `groups()` +
   `groups-updated` event), and add a **Groups** section/tab to signal-ui
   (reuse the avatar `<img>` support from Phase 3).

## Open questions / risks
- Does a linked device get the **master key** during our provisioning flow, or do
  we need a keys-sync request (`SyncMessage::Request` Type::Keys)? Verify what
  `link_device` already captures.
- Storage-service + groups-v2 pull in zkgroup / credential crypto — confirm it
  compiles for `wasm32-wasip2` (the `groups_v2` deps) like the rest of the client.
- Group avatars are separate CDN downloads (not inline like contact avatars), so
  the engine fetches them via `get_attachment`/CDN.

## Building blocks in place
- Contacts + per-ACI persistence (task 67 Phase 3, `repros/signal-engine`,
  `/state/contacts.json`) — group members resolve against these.
- dioxus-canvas `<img>` support for avatars (task 67 Phase 3,
  [[reference_dioxus_taffy_rust_ui]]).
- Transport + persistent executor (`tasks/66`, `tasks/67`,
  [[project_wart_step_executor]]).
