//! The Signal engine behind the `wart:signal/chat` export. A single background
//! task (spawned on the persistent [`wart_step_executor`]) links or resumes, then
//! runs the receive+send loop; `chat.poll-events` advances that task one
//! non-blocking step and drains whatever it produced. State shared between the
//! export functions and the background task lives in a thread-local `Shared`
//! (single-threaded guest ⇒ `Rc`/`RefCell`).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::time::Duration;

use base64::Engine as _;
use futures::future::FutureExt;
use futures::StreamExt;
use rand::SeedableRng;

use libsignal_service::cipher::ServiceCipher;
use libsignal_service::configuration::{
    ServiceConfiguration, ServiceCredentials, SignalServers,
};
use libsignal_service::attachment_cipher::decrypt_in_place;
use libsignal_service::content::ContentBody;
use libsignal_service::groups_v2::{
    decrypt_group, GroupsManager, InMemoryCredentialsCache,
};
use libsignal_service::master_key::{MasterKey, StorageServiceKey};
use libsignal_service::profile_cipher::ProfileCipher;
use libsignal_service::protocol::Aci;
use libsignal_service::zkgroup::profiles::ProfileKey;
use libsignal_service::unidentified_access::UnidentifiedAccess;
use libsignal_service::zkgroup::groups::{GroupMasterKey, GroupSecretParams};
use libsignal_service::messagepipe::Incoming;
use libsignal_service::proto::{
    data_message, manifest_record, receipt_message, storage_record, sync_message,
    AttachmentPointer, DataMessage, GroupContextV2, ReceiptMessage,
};
use libsignal_service::protocol::{
    DeviceId, IdentityKeyPair, IdentityKeyStore, ProtocolAddress, ServiceId,
};
use libsignal_service::ServiceIdExt;
use libsignal_service::provisioning::{
    generate_registration_id, link_device, SecondaryDeviceProvisioning,
};
use libsignal_service::push_service::{PushService, ServiceIds};
use libsignal_service::receiver::MessageReceiver;
use libsignal_service::sender::MessageSender;
use libsignal_service::storage_service::StorageService;
use libsignal_service::websocket::Unidentified;
use uuid::Uuid;

use crate::exports::wart::signal::chat::{
    Attachment, CallState, Contact, Delivery, DeliveryStatus, Event, Group, Message, Profile,
    ReactionUpdate,
};
use crate::call::{self, CallEngine};
use crate::persist;
use crate::store::MemStore;

// ---- shared state ----------------------------------------------------------

/// Routing metadata for a group send: the member ServiceIds and the group's
/// current revision. The group master key is `b64::decode(thread)`. Kept in
/// `Shared` (re-populated on every group fetch) + persisted on `StoredGroup`.
#[derive(Clone)]
struct GroupRoute {
    members: Vec<Uuid>,
    revision: u32,
}

/// A pending receipt to send back to a message's sender: the recipient ACI, the
/// kind (delivery vs read), and the wire timestamps of the messages it covers.
struct ReceiptJob {
    recipient: Uuid,
    read: bool,
    timestamps: Vec<u64>,
}

struct Shared {
    events: RefCell<VecDeque<Event>>,
    history: RefCell<Vec<Message>>,
    /// Outgoing queue: `(thread, text, ts)` — thread routes the wire send; ts is
    /// the message's timestamp, reused as the wire timestamp so delivery/read
    /// receipts (which reference it) match back to this message.
    outbox: RefCell<VecDeque<(String, String, u64)>>,
    state: RefCell<String>,
    next_id: RefCell<u64>,
    /// Contact list from the primary device's address book (contacts-sync).
    contacts: RefCell<Vec<Contact>>,
    /// Set once the initial contacts request has been sent on this connection.
    contacts_requested: Cell<bool>,
    /// Set by `sync_contacts()` → the receive loop re-requests on the next tick.
    resync_contacts: Cell<bool>,
    /// Groups (Groups v2) fetched via the storage service.
    groups: RefCell<Vec<Group>>,
    /// Account master key (from provisioning) — needed for storage service +
    /// groups. `None` until a link captures it (post-task-69).
    master_key: RefCell<Option<Vec<u8>>>,
    /// Set by `sync_groups()` → the receive loop re-fetches groups on the tick.
    resync_groups: Cell<bool>,
    /// Per-group routing (member ACIs + revision), keyed by group id (master key
    /// b64). Used to address `send_message_to_group`.
    group_routes: RefCell<HashMap<String, GroupRoute>>,
    /// This account's own ACI (lowercase uuid), once linked/resumed — lets the UI
    /// label the self thread "Note to Self". Empty until known.
    account_id: RefCell<String>,
    /// Emoji reactions, keyed by the target message's wire timestamp → (reactor
    /// ACI → emoji). Re-derived into the target `message.reactions` string on
    /// change; persisted to reactions.json so they survive a restart.
    reactions: RefCell<HashMap<u64, HashMap<String, String>>>,
    /// Our OWN profile key (32 bytes) from provisioning — fetches+decrypts our
    /// profile. `None` for accounts linked before this was captured (re-link).
    profile_key: RefCell<Option<Vec<u8>>>,
    /// Our own Signal profile (name/avatar/about/phone), once fetched. The UI
    /// shows name+avatar+phone; the rest is carried for future use.
    profile: RefCell<Option<Profile>>,
    /// Set once we've fetched the profile on this connection (fetch once/connect).
    profile_fetched: Cell<bool>,
    /// Set by `sync_profile()` → the receive loop re-fetches on the next tick.
    resync_profile: Cell<bool>,
    /// Receipts to send back (delivery on receive, read on `mark_read`), drained
    /// over the connection in the send tick.
    receipt_outbox: RefCell<Vec<ReceiptJob>>,
    /// Wire timestamps we've already sent a READ receipt for (so re-opening a
    /// thread doesn't re-ack everything). In-memory — resending on restart is
    /// harmless (idempotent on the peer).
    read_acked: RefCell<HashSet<u64>>,
    /// 1:1 voice call (Phase 2b-ii). The export-thread `place/accept/hangup-call`
    /// funcs enqueue intents; the background `run()` loop drives the `CallEngine`
    /// and mirrors its state back here for the synchronous `call-status`/`call-peer`.
    call_intents: RefCell<VecDeque<CallIntent>>,
    /// Latest call state, mirrored for the synchronous `call-status` getter.
    call_state: Cell<CallState>,
    /// Peer ACI of the current/ringing call ("" when idle).
    call_peer: RefCell<String>,
}

/// A call action requested by the UI (`place/accept/hangup-call`), processed by
/// the background loop on its next tick.
enum CallIntent {
    Place(String),
    Accept,
    Hangup,
}

/// Send each [`wart_call::signal::CallSignal`] to `$peer` (an ACI string) as a
/// `CallMessage` over the Signal channel. A macro, not a fn, to dodge naming
/// `MessageSender`'s store bounds; `$sender` is an already-borrowed `&mut MessageSender`.
macro_rules! send_signals {
    ($sender:expr, $peer:expr, $sigs:expr) => {{
        if let Ok(uuid) = Uuid::parse_str(&$peer) {
            let recipient = ServiceId::Aci(uuid.into());
            for sig in $sigs {
                // Surface the trickled TURN relay candidate (Phase A device check).
                if let wart_call::signal::CallSignal::Ice { opaque, .. } = &sig {
                    if let Ok(Some(line)) = wart_call::signal::decode_ice_candidate(opaque) {
                        let kind = if line.contains("typ relay") { "RELAY" } else { "host" };
                        dbg_line(&format!("trickle {kind} candidate -> {line}"));
                    }
                }
                let cm = call::signal_to_call_message(&sig);
                let _ = $sender
                    .send_message(&recipient, None, cm, now_ms(), false, false)
                    .await;
            }
        }
    }};
}

fn to_wit_call_state(s: wart_call::signal::CallState) -> CallState {
    use wart_call::signal::CallState as W;
    match s {
        W::Outgoing => CallState::Outgoing,
        W::Ringing => CallState::Ringing,
        W::Connecting => CallState::Connecting,
        W::Connected => CallState::Connected,
        W::Ended => CallState::Ended,
    }
}

/// Fetch Signal's calling relays and build a TURN config from the first usable UDP
/// relay (so a call connects across NAT). `None` on any failure — calls then fall
/// back to host candidates only.
/// Append a line to `/state/calldbg.log` — guest `log::` doesn't reach logcat on
/// this build, so call/TURN tracing goes to a file we read over adb.
fn dbg_line(s: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/state/calldbg.log")
    {
        let _ = writeln!(f, "{} {}", now_ms(), s);
    }
}

async fn fetch_turn(push: &PushService) -> Option<wart_call::turn::TurnConfig> {
    let mut p = push.clone();
    let resp = match p.get_calling_relays().await {
        Ok(r) => r,
        Err(e) => {
            dbg_line(&format!("turn: get_calling_relays FAILED: {e}"));
            return None;
        }
    };
    let tc = resp.relays.iter().find_map(call::turn_config_from);
    match &tc {
        Some(c) => dbg_line(&format!("turn: relay {} realm={:?}", c.turn_serv_addr, c.realm)),
        None => dbg_line(&format!(
            "turn: NO usable UDP relay among {} set(s); first urls={:?}",
            resp.relays.len(),
            resp.relays.first().map(|r| (&r.urls, &r.urls_with_ips)),
        )),
    }
    tc
}

/// Resolve a peer's serialized identity public key from the protocol store — the
/// 33-byte key ringrtc's call HKDF binds to. `None` if we hold no identity for them
/// yet (no session). Looks up device 1 (the identity key is account-level).
async fn peer_identity_key(store: &MemStore, aci: &str) -> Option<Vec<u8>> {
    let uuid = Uuid::parse_str(aci).ok()?;
    let device = DeviceId::try_from(1u32).ok()?;
    let addr = ServiceId::Aci(uuid.into()).to_protocol_address(device).ok()?;
    // Try device 1, then scan all devices (identity key is account-level).
    let ik = match store.get_identity(&addr).await.ok().flatten() {
        Some(ik) => Some(ik),
        None => store.identity_for_name(addr.name()),
    };
    dbg_line(&format!(
        "peer_identity {} ({}) -> {}",
        aci,
        addr.name(),
        if ik.is_some() { "FOUND" } else { "MISSING (no session?)" },
    ));
    Some(ik?.serialize().to_vec())
}

/// Mirror the driver's call state into `Shared` (for the synchronous
/// `call-status`/`call-peer` getters). Returns whether the state changed.
fn sync_call_state(shared: &Shared, ce: &CallEngine) -> bool {
    let wit = ce.state().map(to_wit_call_state).unwrap_or(CallState::Idle);
    *shared.call_peer.borrow_mut() = ce.peer();
    let changed = shared.call_state.get() != wit;
    shared.call_state.set(wit);
    changed
}

impl Shared {
    fn push_event(&self, e: Event) {
        self.events.borrow_mut().push_back(e);
    }

    fn set_state(&self, s: impl Into<String>) {
        *self.state.borrow_mut() = s.into();
    }

    fn next_id(&self) -> u64 {
        let mut n = self.next_id.borrow_mut();
        *n += 1;
        *n
    }

    /// Record a message in history and emit it as a live event.
    fn add_message(&self, m: Message) {
        self.history.borrow_mut().push(m.clone());
        self.push_event(Event::Message(m));
    }

    /// Advance an outgoing message's delivery state (matched by wire timestamp),
    /// monotonically. Persists the new state + emits a `status-changed` event so
    /// the UI updates its checkmark. No-op if the message is unknown or the new
    /// state isn't an advance.
    fn set_status(&self, ts: u64, new: Delivery) {
        let updated = {
            let mut hist = self.history.borrow_mut();
            match hist.iter_mut().find(|m| m.ts == ts && m.outgoing) {
                Some(m) if rank(new) > rank(m.status) => {
                    m.status = new;
                    Some(m.id)
                },
                _ => None,
            }
        };
        if let Some(id) = updated {
            persist::set_status(ts, status_code(new));
            self.push_event(Event::StatusChanged(DeliveryStatus { id, status: new }));
        }
    }

    /// Apply an emoji reaction (add, or `remove`) by one reactor to the message
    /// with wire timestamp `target_ts`. Updates the reactor→emoji map, re-derives
    /// the target's `reactions` string, persists, and emits `reaction-changed` so
    /// the UI re-renders. No-op (but still persisted) if the target isn't in
    /// history yet — the overlay on the next `init` will pick it up.
    fn apply_reaction(&self, target_ts: u64, reactor: String, emoji: String, remove: bool) {
        {
            let mut all = self.reactions.borrow_mut();
            let per = all.entry(target_ts).or_default();
            if remove {
                per.remove(&reactor);
            } else {
                per.insert(reactor, emoji);
            }
            if per.is_empty() {
                all.remove(&target_ts);
            }
        }
        let joined = self.reactions_string(target_ts);
        let authors = self.reactions.borrow().get(&target_ts).cloned().unwrap_or_default();
        persist::set_reactions(target_ts, &authors);
        let id = {
            let mut hist = self.history.borrow_mut();
            hist.iter_mut().find(|m| m.ts == target_ts).map(|m| {
                m.reactions = joined.clone();
                m.id
            })
        };
        if let Some(id) = id {
            self.push_event(Event::ReactionChanged(ReactionUpdate { id, reactions: joined }));
        }
    }

    /// The reactions on `target_ts` rendered for display (see
    /// [`reactions_to_string`]). Empty = none.
    fn reactions_string(&self, target_ts: u64) -> String {
        self.reactions
            .borrow()
            .get(&target_ts)
            .map(reactions_to_string)
            .unwrap_or_default()
    }
}

/// Render a reactor→emoji map for display: each distinct emoji once, with a
/// trailing count when more than one reactor used it (so a group's multiple
/// reactions all show — e.g. "❤️3 👍"). Space-joined.
fn reactions_to_string(per: &HashMap<String, String>) -> String {
    let mut counts: Vec<(String, u32)> = Vec::new();
    for emoji in per.values() {
        match counts.iter_mut().find(|(e, _)| e == emoji) {
            Some((_, n)) => *n += 1,
            None => counts.push((emoji.clone(), 1)),
        }
    }
    counts
        .into_iter()
        .map(|(e, n)| if n > 1 { format!("{e}{n}") } else { e })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Monotonic ordering of delivery states (higher = further along).
fn rank(d: Delivery) -> u8 {
    match d {
        Delivery::Sending => 0,
        Delivery::Sent => 1,
        Delivery::Delivered => 2,
        Delivery::Read => 3,
    }
}

fn status_code(d: Delivery) -> u8 {
    rank(d)
}

fn status_from_code(c: u8) -> Delivery {
    match c {
        0 => Delivery::Sending,
        2 => Delivery::Delivered,
        3 => Delivery::Read,
        _ => Delivery::Sent,
    }
}

thread_local! {
    static SHARED: RefCell<Option<Rc<Shared>>> = const { RefCell::new(None) };
}

fn shared() -> Option<Rc<Shared>> {
    SHARED.with(|s| s.borrow().clone())
}

// ---- export entry points (called from the Guest impl in lib.rs) ------------

pub fn init() {
    if shared().is_some() {
        return; // idempotent
    }
    wart_step_executor::init();

    // Preload persisted history, assigning stable ids. Delivery state defaults to
    // `sent` (a persisted message was at least sent/received), overlaid by the
    // delivered/read states saved in statuses.json (keyed by wire ts).
    let statuses = persist::load_statuses();
    // Persisted reactions, keyed by the target message's wire ts (reactor→emoji).
    let reactions = persist::load_reactions();
    let mut history = Vec::new();
    let mut id = 0u64;
    for m in persist::load_messages() {
        id += 1;
        let status = statuses
            .get(&m.ts)
            .map(|c| status_from_code(*c))
            .unwrap_or(Delivery::Sent);
        // Overlay any persisted reactions on this message.
        let reaction_str = reactions.get(&m.ts).map(reactions_to_string).unwrap_or_default();
        // Reload persisted attachment bytes from /state/att/ (skip any missing).
        let attachments = m
            .attachments
            .iter()
            .filter_map(|a| {
                persist::load_attachment(&a.file).map(|data| Attachment {
                    content_type: a.content_type.clone(),
                    data,
                    width: a.width,
                    height: a.height,
                })
            })
            .collect();
        history.push(Message {
            id,
            sender: m.from,
            text: m.text,
            ts: m.ts,
            outgoing: m.outgoing,
            thread: m.thread,
            status,
            reactions: reaction_str,
            attachments,
        });
    }

    // Preload persisted contacts (avatar base64 → bytes).
    let contacts: Vec<Contact> = persist::load_contacts()
        .into_iter()
        .map(|c| Contact {
            id: c.id,
            name: c.name,
            phone: c.phone,
            inbox_position: c.inbox_position,
            avatar: c.avatar_b64.and_then(|b| b64().decode(b).ok()),
        })
        .collect();

    // Persisted groups → WIT records (for display) + routing table (for sends).
    let stored_groups = persist::load_groups();
    let mut group_routes: HashMap<String, GroupRoute> = HashMap::new();
    for g in &stored_groups {
        group_routes.insert(
            g.id.clone(),
            GroupRoute {
                members: g.member_ids.iter().filter_map(|s| Uuid::parse_str(s).ok()).collect(),
                revision: g.revision,
            },
        );
    }
    let groups: Vec<Group> = stored_groups
        .into_iter()
        .map(|g| Group {
            id: g.id,
            title: g.title,
            members: g.members,
            avatar: g.avatar_b64.as_ref().and_then(|b| b64().decode(b).ok()),
        })
        .collect();

    // Preload our own profile (if fetched on a prior run).
    let profile: Option<Profile> = persist::load_profile().map(to_wit_profile);

    let s = Rc::new(Shared {
        events: RefCell::new(VecDeque::new()),
        history: RefCell::new(history),
        outbox: RefCell::new(VecDeque::new()),
        state: RefCell::new("starting".to_string()),
        next_id: RefCell::new(id),
        contacts: RefCell::new(contacts),
        contacts_requested: Cell::new(false),
        resync_contacts: Cell::new(false),
        groups: RefCell::new(groups),
        master_key: RefCell::new(None),
        resync_groups: Cell::new(false),
        group_routes: RefCell::new(group_routes),
        account_id: RefCell::new(String::new()),
        reactions: RefCell::new(reactions),
        profile_key: RefCell::new(None),
        profile: RefCell::new(profile),
        profile_fetched: Cell::new(false),
        resync_profile: Cell::new(false),
        receipt_outbox: RefCell::new(Vec::new()),
        read_acked: RefCell::new(HashSet::new()),
        call_intents: RefCell::new(VecDeque::new()),
        call_state: Cell::new(CallState::Idle),
        call_peer: RefCell::new(String::new()),
    });
    SHARED.with(|slot| *slot.borrow_mut() = Some(s));

    // Detach: the root task must outlive this call (it cancels on drop).
    wart_step_executor::spawn(run()).detach();
}

pub fn poll_events() -> Vec<Event> {
    if shared().is_none() {
        return Vec::new();
    }
    // Advance the background task(s) without blocking the frame.
    wart_step_executor::step();
    shared()
        .map(|s| s.events.borrow_mut().drain(..).collect())
        .unwrap_or_default()
}

/// This account's own ACI (lowercase uuid), or "" if not yet linked. Lets the UI
/// recognise the Note-to-Self thread.
pub fn account_id() -> String {
    shared().map(|s| s.account_id.borrow().clone()).unwrap_or_default()
}

/// Our own Signal profile (name/avatar/about/phone), or an all-empty record until
/// it's been fetched (a `profile-updated` event fires when it arrives).
pub fn my_profile() -> Profile {
    shared()
        .and_then(|s| s.profile.borrow().clone())
        .unwrap_or_else(|| Profile {
            given_name: String::new(),
            family_name: String::new(),
            about: String::new(),
            about_emoji: String::new(),
            phone: String::new(),
            avatar: None,
        })
}

/// Request a fresh fetch of our own profile (also done once on connect).
pub fn sync_profile() {
    if let Some(s) = shared() {
        s.resync_profile.set(true);
    }
}

pub fn place_call(thread: String) -> Result<(), String> {
    let s = shared().ok_or("engine not initialized")?;
    if thread.trim().is_empty() {
        return Err("no call peer".to_string());
    }
    if s.call_state.get() != CallState::Idle {
        return Err("a call is already active".to_string());
    }
    // Driven on the next background tick (needs the network sender + UDP/audio).
    s.call_intents.borrow_mut().push_back(CallIntent::Place(thread));
    Ok(())
}

pub fn accept_call() {
    if let Some(s) = shared() {
        s.call_intents.borrow_mut().push_back(CallIntent::Accept);
    }
}

pub fn hangup_call() {
    if let Some(s) = shared() {
        s.call_intents.borrow_mut().push_back(CallIntent::Hangup);
    }
}

pub fn call_status() -> CallState {
    shared().map(|s| s.call_state.get()).unwrap_or(CallState::Idle)
}

pub fn call_peer() -> String {
    shared().map(|s| s.call_peer.borrow().clone()).unwrap_or_default()
}

pub fn send(thread: String, text: String) -> Result<(), String> {
    let shared = shared().ok_or("engine not initialized")?;
    if text.trim().is_empty() {
        return Err("empty message".to_string());
    }
    // Local echo into history + live feed; the actual wire send happens in the
    // background task within ~200 ms (see `receive_and_send`), routed by `thread`.
    let ts = now_ms();
    let msg = Message {
        id: shared.next_id(),
        sender: "me".to_string(),
        text: text.clone(),
        ts,
        outgoing: true,
        thread: thread.clone(),
        status: Delivery::Sending,
        reactions: String::new(),
        attachments: Vec::new(),
    };
    let _ = persist::append_message(&to_stored(&msg));
    shared.add_message(msg);
    // Carry `ts` so the wire send stamps the DataMessage with it — receipts then
    // reference this exact ts and match back via `set_status`.
    shared.outbox.borrow_mut().push_back((thread, text, ts));
    Ok(())
}

/// Mark `thread` read: enqueue a read receipt for every received (incoming)
/// message in it we haven't acked yet, grouped by sender (so group messages ack
/// to each member). Sent over the connection on the next tick.
pub fn mark_read(thread: String) {
    let Some(shared) = shared() else { return };
    let mut by_sender: HashMap<Uuid, Vec<u64>> = HashMap::new();
    {
        let hist = shared.history.borrow();
        let mut acked = shared.read_acked.borrow_mut();
        for m in hist.iter() {
            if m.thread == thread && !m.outgoing && !acked.contains(&m.ts) {
                if let Ok(uuid) = Uuid::parse_str(&m.sender) {
                    by_sender.entry(uuid).or_default().push(m.ts);
                    acked.insert(m.ts);
                }
            }
        }
    }
    if by_sender.is_empty() {
        return;
    }
    let mut ob = shared.receipt_outbox.borrow_mut();
    for (recipient, timestamps) in by_sender {
        ob.push(ReceiptJob { recipient, read: true, timestamps });
    }
}

pub fn history() -> Vec<Message> {
    shared()
        .map(|s| s.history.borrow().clone())
        .unwrap_or_default()
}

pub fn contacts() -> Vec<Contact> {
    shared()
        .map(|s| s.contacts.borrow().clone())
        .unwrap_or_default()
}

pub fn sync_contacts() {
    if let Some(s) = shared() {
        s.resync_contacts.set(true);
    }
}

pub fn groups() -> Vec<Group> {
    shared().map(|s| s.groups.borrow().clone()).unwrap_or_default()
}

pub fn sync_groups() {
    if let Some(s) = shared() {
        s.resync_groups.set(true);
    }
}

pub fn state() -> String {
    shared()
        .map(|s| s.state.borrow().clone())
        .unwrap_or_else(|| "uninitialized".to_string())
}

// ---- background task: link/resume, then receive + send ---------------------

async fn run() {
    let shared = match shared() {
        Some(s) => s,
        None => return,
    };

    let setup = match persist::load_account() {
        Some(account) => resume(&shared, account),
        None => link(&shared).await,
    };
    let (store, credentials, aci, device_id) = match setup {
        Ok(v) => v,
        Err(e) => {
            shared.set_state(format!("error: {e}"));
            shared.push_event(Event::Disconnected);
            return;
        },
    };
    *shared.account_id.borrow_mut() = aci.to_string();

    if let Err(e) =
        receive_and_send(&shared, store, credentials, aci, device_id).await
    {
        shared.set_state(format!("error: {e}"));
        shared.push_event(Event::Disconnected);
    }
}

/// First-run link as a secondary device. Emits `link-url` (the QR payload) then
/// `linked`, persisting the account so later runs `resume` instead.
async fn link(
    shared: &Rc<Shared>,
) -> Result<(MemStore, ServiceCredentials, Uuid, u32), String> {
    shared.set_state("linking");

    let password = gen_password();
    let aci_store = MemStore::new(
        IdentityKeyPair::generate(&mut seed_rng()),
        generate_registration_id(&mut seed_rng()),
    );
    let mut pni_store = MemStore::new(
        IdentityKeyPair::generate(&mut seed_rng()),
        generate_registration_id(&mut seed_rng()),
    );
    let mut aci_for_task = aci_store.clone();

    let push =
        PushService::new(SignalServers::Production, None, "wart-signal-engine");
    let (tx, mut rx) = futures::channel::mpsc::channel(1);
    let pw = password.clone();
    let task = wart_step_executor::spawn(async move {
        let mut csprng = seed_rng();
        link_device(
            &mut aci_for_task,
            &mut pni_store,
            &mut csprng,
            push,
            &pw,
            "wart",
            tx,
        )
        .await
    });

    let mut registration = None;
    while let Some(step) = rx.next().await {
        match step {
            SecondaryDeviceProvisioning::Url(url) => {
                shared.push_event(Event::LinkUrl(url.to_string()));
            },
            SecondaryDeviceProvisioning::NewDeviceRegistration(reg) => {
                registration = Some(reg);
            },
        }
    }
    task.await.map_err(|e| format!("link: {e}"))?;
    let reg = registration.ok_or("no registration received")?;

    let identity =
        IdentityKeyPair::new(reg.aci_public_key, reg.aci_private_key);
    aci_store.set_identity(identity, reg.registration_id);

    // Capture the account master key (for the storage service + groups, task 69).
    // Modern primaries send the Account Entropy Pool, not the deprecated
    // master_key field — derive the master key from the AEP in that case
    // (`derive_svr_key` = the `…_SVR_MASTER_KEY` HKDF, i.e. the master key).
    let master_key_bytes: Option<Vec<u8>> = reg.master_key.clone().or_else(|| {
        reg.account_entropy_pool
            .as_ref()
            .map(|aep| aep.derive_svr_key().to_vec())
    });
    *shared.master_key.borrow_mut() = master_key_bytes.clone();
    // Capture our OWN profile key — only available here, at link, in the
    // provisioning message. Needed to fetch + decrypt our own profile (and the
    // basis for any future "fetch contacts' profiles", though those keys come
    // from the storage service, not here).
    let profile_key_bytes = reg.profile_key.get_bytes().to_vec();
    *shared.profile_key.borrow_mut() = Some(profile_key_bytes.clone());
    let account = persist::Account {
        aci: reg.service_ids.aci,
        pni: reg.service_ids.pni,
        number: reg.phone_number.to_string(),
        password: password.clone(),
        device_id: u32::from(reg.device_id),
        registration_id: reg.registration_id,
        identity_b64: b64().encode(identity.serialize()),
        master_key_b64: master_key_bytes.as_ref().map(|mk| b64().encode(mk)),
        profile_key_b64: Some(b64().encode(&profile_key_bytes)),
    };
    persist::save_account(&account)
        .map_err(|e| format!("save account: {e}"))?;
    persist::save_snapshot(&aci_store.snapshot_bytes())
        .map_err(|e| format!("save snapshot: {e}"))?;

    shared.set_state("linked");
    shared.push_event(Event::Linked(reg.phone_number.to_string()));

    let credentials = ServiceCredentials {
        aci: Some(reg.service_ids.aci),
        pni: Some(reg.service_ids.pni),
        phonenumber: reg.phone_number,
        password: Some(password),
        device_id: Some(reg.device_id),
    };
    Ok((aci_store, credentials, reg.service_ids.aci, u32::from(reg.device_id)))
}

/// Re-authenticate a previously linked device from the persisted account.
fn resume(
    shared: &Rc<Shared>,
    account: persist::Account,
) -> Result<(MemStore, ServiceCredentials, Uuid, u32), String> {
    // Restore the master key (for groups) if this account was linked post-task-69.
    *shared.master_key.borrow_mut() =
        account.master_key_b64.as_ref().and_then(|b| b64().decode(b).ok());
    // Restore our own profile key (for the profile fetch) if captured at link.
    *shared.profile_key.borrow_mut() =
        account.profile_key_b64.as_ref().and_then(|b| b64().decode(b).ok());

    let identity = IdentityKeyPair::try_from(
        b64()
            .decode(&account.identity_b64)
            .map_err(|e| format!("identity b64: {e}"))?
            .as_slice(),
    )
    .map_err(|e| format!("identity decode: {e}"))?;

    let store = MemStore::new(identity, account.registration_id);
    if let Some(snap) = persist::load_snapshot() {
        store.load_into(&snap)?;
    }

    let credentials = ServiceCredentials {
        aci: Some(account.aci),
        pni: Some(account.pni),
        phonenumber: account
            .number
            .parse()
            .map_err(|_| "bad saved phone number".to_string())?,
        password: Some(account.password),
        device_id: Some(
            DeviceId::try_from(account.device_id)
                .map_err(|e| format!("device id: {e}"))?,
        ),
    };
    Ok((store, credentials, account.aci, account.device_id))
}

/// Open the authenticated message socket, then loop: receive+decrypt envelopes
/// and, on a short timer, drain the outbox over the same connection.
async fn receive_and_send(
    shared: &Rc<Shared>,
    store: MemStore,
    credentials: ServiceCredentials,
    aci: Uuid,
    device_id: u32,
) -> Result<(), String> {
    let pni = credentials.pni.unwrap_or(aci);
    // Our phone number (for the profile, which carries no phone) — captured
    // before `credentials` is consumed by the message pipe.
    let phone = credentials.phonenumber.to_string();
    let push = PushService::new(
        SignalServers::Production,
        Some(credentials.clone()),
        "wart-signal-engine",
    );
    let mut receiver = MessageReceiver::new(push.clone());
    let pipe = receiver
        .create_message_pipe(credentials, false)
        .await
        .map_err(|e| format!("open message ws: {e}"))?;

    let trust_roots = ServiceConfiguration::from(SignalServers::Production)
        .unidentified_sender_trust_roots;
    let local_address = ProtocolAddress::new(
        aci.to_string(),
        DeviceId::try_from(device_id).map_err(|e| format!("device id: {e}"))?,
    );
    let mut cipher =
        ServiceCipher::new(store.clone(), trust_roots, local_address);
    let mut csprng = seed_rng();

    // Build the sender once, reusing the pipe's identified socket. If the
    // unidentified socket can't be opened, receiving still works; sends are
    // disabled and reported via `state`.
    let device =
        DeviceId::try_from(device_id).map_err(|e| format!("device id: {e}"))?;
    let mut sender = match push
        .clone()
        .ws::<Unidentified>("/v1/websocket/", "/v1/keepalive", &[], None)
        .await
    {
        Ok(unidentified_ws) => Some(MessageSender::new(
            pipe.ws(),
            unidentified_ws,
            push.clone(),
            cipher.clone(),
            store.clone(),
            aci,
            pni,
            store.identity(),
            None,
            device,
        )),
        Err(e) => {
            shared.set_state(format!("send unavailable: {e}"));
            None
        },
    };
    let self_id = ServiceId::Aci(aci.into());

    shared.set_state("connected");
    shared.push_event(Event::Connected);

    // Ask the primary device for its contact list — the response arrives later as
    // a SyncMessage on the stream (handled below) and is persisted to /state.
    if let Some(s) = sender.as_mut() {
        match s
            .send_sync_message_request(&self_id, sync_message::request::Type::Contacts)
            .await
        {
            Ok(_) => shared.contacts_requested.set(true),
            Err(e) => shared.set_state(format!("contacts request failed: {e}")),
        }
    }

    // Groups (Groups v2): direct fetch via the storage service (no primary sync),
    // in the background so it doesn't block receiving. Needs the master key, which
    // only a post-task-69 link captured — else it's skipped (state notes it).
    spawn_group_fetch(shared, &push, aci, pni);

    // Grab a profile-fetch websocket handle BEFORE `pipe.stream()` consumes the
    // pipe (the profile fetch in the tick below uses it).
    let mut profile_ws = pipe.ws();
    let mut stream = Box::pin(pipe.stream());
    // Phase 2b-ii/3: the 1:1 voice-call driver. Our serialized identity public key
    // binds the call's SRTP keys (the peer's is resolved per call from the store).
    // While a call is active the loop ticks at ~10 ms so the audio/UDP pump keeps
    // up; otherwise the normal 200 ms message cadence.
    let mut call_engine = CallEngine::new();
    let my_identity: Vec<u8> = store.identity().identity_key().serialize().to_vec();
    loop {
        let tick_ms = if call_engine.is_active() { 10 } else { 200 };
        let tick = wart_step_executor::sleep(Duration::from_millis(tick_ms));
        futures::select! {
            item = stream.next().fuse() => match item {
                Some(Ok(Incoming::Envelope(env))) => {
                    let content = match cipher.open_envelope(env, &mut csprng).await {
                        Ok(c) => c,
                        Err(e) => {
                            shared.set_state(format!("decrypt error: {e}"));
                            None
                        },
                    };
                    // The ratchet may have advanced — persist after every envelope.
                    let _ = persist::save_snapshot(&store.snapshot_bytes());
                    if let Some(content) = content {
                        // Contacts-sync response: download + decrypt the blob,
                        // store the users in /state, emit `contacts-updated`.
                        if let ContentBody::SynchronizeMessage(sm) = &content.body {
                            if let Some(blob) = &sm.contacts {
                                if let Err(e) =
                                    apply_contacts(&shared, &mut receiver, blob).await
                                {
                                    shared.set_state(format!("contacts fetch: {e}"));
                                }
                            }
                        }
                        // Delivery/read receipt from a recipient: advance the
                        // matching outgoing message(s) by their wire timestamp.
                        if let ContentBody::ReceiptMessage(r) = &content.body {
                            let new = if r.r#type == Some(receipt_message::Type::Delivery as i32) {
                                Some(Delivery::Delivered)
                            } else if r.r#type == Some(receipt_message::Type::Read as i32) {
                                Some(Delivery::Read)
                            } else {
                                None // viewed (stories) — ignore
                            };
                            if let Some(new) = new {
                                for &ts in &r.timestamp {
                                    shared.set_status(ts, new);
                                }
                            }
                        }
                        // Emoji reaction (someone liked/reacted to a message):
                        // attach it to its target (matched by the target's wire
                        // sent timestamp) and emit `reaction-changed`.
                        if let Some(r) = reaction_of(&content.body) {
                            if let Some(target_ts) = r.target_sent_timestamp {
                                let emoji = r.emoji.clone().unwrap_or_default();
                                let remove = r.remove.unwrap_or(false);
                                let reactor = content.metadata.sender.raw_uuid().to_string();
                                if remove || !emoji.is_empty() {
                                    shared.apply_reaction(target_ts, reactor, emoji, remove);
                                }
                            }
                        }
                        // 1:1 voice-call signaling (Phase 2b-ii): feed inbound
                        // CallMessages to the driver — start ringing on a fresh
                        // offer, else apply answer/ICE/hangup; send any reply (Busy).
                        if let ContentBody::CallMessage(cm) = &content.body {
                            let from = content.metadata.sender.raw_uuid().to_string();
                            // The offerer's identity key (caller) — from the exact
                            // address the message came from (its identity is stored).
                            let sender_identity = match content
                                .metadata
                                .sender
                                .to_protocol_address(content.metadata.sender_device)
                                .ok()
                            {
                                Some(addr) => store
                                    .get_identity(&addr)
                                    .await
                                    .ok()
                                    .flatten()
                                    .map(|ik| ik.serialize().to_vec())
                                    .unwrap_or_default(),
                                None => Vec::new(),
                            };
                            // Fetch TURN only for a likely incoming offer (no active
                            // call) — not for mid-call Answer/Ice.
                            let turn = if call_engine.is_active() {
                                None
                            } else {
                                fetch_turn(&push).await
                            };
                            let (replies, incoming) = call_engine.on_call_message(
                                cm, &from, &my_identity, &sender_identity, turn,
                            );
                            if let Some(sender) = sender.as_mut() {
                                send_signals!(sender, from, replies);
                            }
                            if let Some(peer) = incoming {
                                shared.push_event(Event::CallIncoming(peer));
                            }
                            sync_call_state(&shared, &call_engine);
                        }
                        let (body, outgoing, thread_override, wire_ts) =
                            extract(&content.body);
                        // Download + decrypt image attachments (v1: images only,
                        // so we don't pull large video/files). Cloned out of the
                        // borrow so we can await without holding `content`.
                        let ptrs: Vec<AttachmentPointer> = attachments_of(&content.body)
                            .iter()
                            .filter(|p| p.content_type.as_deref().is_some_and(|c| c.starts_with("image/")))
                            .cloned()
                            .collect();
                        let mut attachments = Vec::new();
                        for ptr in &ptrs {
                            if let Some(a) = fetch_attachment(&push, ptr).await {
                                attachments.push(a);
                            }
                        }
                        if body.is_some() || !attachments.is_empty() {
                            let text = body.unwrap_or_default();
                            // Sender: my own messages (synced from another device)
                            // show as "me"; otherwise the peer's ACI (UI → name).
                            let peer = content.metadata.sender.raw_uuid().to_string();
                            let sender = if outgoing {
                                "me".to_string()
                            } else {
                                peer.clone()
                            };
                            // Thread: group (from the message) or the 1:1 peer.
                            let thread = thread_override.unwrap_or(peer);
                            // Use the message's own send timestamp (so reactions —
                            // which reference it — match, and the time shown is when
                            // it was SENT, not received). Fall back to local now.
                            let ts = wire_ts.unwrap_or_else(now_ms);
                            // Reactions that arrived before this message did.
                            let reactions = shared.reactions_string(ts);
                            // Ack delivery back to the sender (not for our own
                            // synced-sent messages) — the peer sees one checkmark.
                            if !outgoing {
                                shared.receipt_outbox.borrow_mut().push(ReceiptJob {
                                    recipient: content.metadata.sender.raw_uuid(),
                                    read: false,
                                    timestamps: vec![ts],
                                });
                            }
                            let msg = Message {
                                id: shared.next_id(),
                                sender,
                                text,
                                ts,
                                outgoing,
                                thread,
                                status: Delivery::Sent,
                                reactions,
                                attachments,
                            };
                            let _ = persist::append_message(&to_stored(&msg));
                            shared.add_message(msg);
                        }
                    }
                },
                Some(Ok(Incoming::QueueEmpty)) => {},
                Some(Err(e)) => return Err(format!("message pipe: {e}")),
                None => {
                    shared.push_event(Event::Disconnected);
                    return Ok(());
                },
            },
            _ = tick.fuse() => {
                // 1:1 voice call (Phase 2b-ii): process UI intents, then pump the
                // active call (UDP + audio) and ship any signaling it emits. Runs
                // before the message outbox so audio stays low-latency.
                {
                    let intents: Vec<CallIntent> =
                        shared.call_intents.borrow_mut().drain(..).collect();
                    for intent in intents {
                        let (peer, sigs) = match intent {
                            CallIntent::Place(thread) => {
                                let call_id = now_ms();
                                // Resolve the callee's identity key (need a session).
                                match peer_identity_key(&store, &thread).await {
                                    Some(peer_id) => {
                                      let turn = fetch_turn(&push).await;
                                      match call_engine.place(
                                        call_id, my_identity.clone(), peer_id, thread, turn,
                                    ) {
                                        Ok(sigs) => (call_engine.peer(), sigs),
                                        Err(e) => {
                                            shared.set_state(format!("call: {e}"));
                                            (String::new(), Vec::new())
                                        }
                                      }
                                    }
                                    None => {
                                        shared.set_state(
                                            "call: no identity for peer (message them first)",
                                        );
                                        (String::new(), Vec::new())
                                    }
                                }
                            }
                            CallIntent::Accept => (call_engine.peer(), call_engine.accept()),
                            CallIntent::Hangup => (call_engine.peer(), call_engine.hangup()),
                        };
                        if let Some(sender) = sender.as_mut() {
                            send_signals!(sender, peer, sigs);
                        }
                    }
                    if call_engine.is_active() {
                        let peer = call_engine.peer();
                        let (sigs, _) = call_engine.tick(std::time::Instant::now());
                        if let Some(sender) = sender.as_mut() {
                            send_signals!(sender, peer, sigs);
                        }
                    }
                    if sync_call_state(&shared, &call_engine) {
                        shared.push_event(Event::CallStateChanged(shared.call_state.get()));
                    }
                }
                if let Some(sender) = sender.as_mut() {
                    let pending: Vec<(String, String, u64)> =
                        shared.outbox.borrow_mut().drain(..).collect();
                    for (thread, text, ts) in pending {
                        // Stamp the wire message with the message's own `ts` so
                        // delivery/read receipts (which echo it) match back.
                        let route = shared.group_routes.borrow().get(&thread).cloned();
                        if let Some(route) = route {
                            let master_key = b64().decode(&thread).unwrap_or_default();
                            let dm = DataMessage {
                                body: Some(text),
                                timestamp: Some(ts),
                                group_v2: Some(GroupContextV2 {
                                    master_key: Some(master_key),
                                    revision: Some(route.revision),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            };
                            let recipients: Vec<(ServiceId, Option<UnidentifiedAccess>, bool)> =
                                route
                                    .members
                                    .iter()
                                    .filter(|u| **u != aci)
                                    .map(|u| (ServiceId::Aci((*u).into()), None, false))
                                    .collect();
                            let results = sender
                                .send_message_to_group(&recipients, dm, ts, false)
                                .await;
                            if results.iter().any(|r| r.is_ok()) {
                                shared.set_status(ts, Delivery::Sent);
                            }
                            let _ = persist::save_snapshot(&store.snapshot_bytes());
                        } else {
                            // 1:1 thread = peer ACI (empty/unparseable → self).
                            let recipient = Uuid::parse_str(&thread)
                                .map(|u| ServiceId::Aci(u.into()))
                                .unwrap_or(self_id);
                            let dm = DataMessage {
                                body: Some(text),
                                timestamp: Some(ts),
                                ..Default::default()
                            };
                            match sender
                                .send_message(&recipient, None, dm, ts, false, false)
                                .await
                            {
                                Ok(_) => {
                                    shared.set_status(ts, Delivery::Sent);
                                    let _ = persist::save_snapshot(
                                        &store.snapshot_bytes(),
                                    );
                                },
                                Err(e) => shared.set_state(format!("send error: {e}")),
                            }
                        }
                    }
                    // Drain pending receipts (delivery on receive, read on open):
                    // one ReceiptMessage per recipient, echoing the message ts so
                    // the peer's client matches it back to its sent message.
                    let receipts: Vec<ReceiptJob> =
                        shared.receipt_outbox.borrow_mut().drain(..).collect();
                    for job in receipts {
                        let kind = if job.read {
                            receipt_message::Type::Read
                        } else {
                            receipt_message::Type::Delivery
                        };
                        let rm = ReceiptMessage {
                            r#type: Some(kind as i32),
                            timestamp: job.timestamps,
                        };
                        let recipient = ServiceId::Aci(job.recipient.into());
                        let _ = sender
                            .send_message(&recipient, None, rm, now_ms(), false, false)
                            .await;
                    }
                    // On-demand contacts refresh (sync_contacts()).
                    if shared.resync_contacts.replace(false) {
                        let _ = sender
                            .send_sync_message_request(
                                &self_id,
                                sync_message::request::Type::Contacts,
                            )
                            .await;
                    }
                    // Fetch our own profile once per connection (or on
                    // sync_profile()). Needs the profile key (captured at link).
                    let want_profile = !shared.profile_fetched.replace(true)
                        || shared.resync_profile.replace(false);
                    if want_profile {
                        let pk = shared.profile_key.borrow().clone();
                        if let Some(pk) = pk {
                            if let Some(p) =
                                fetch_own_profile(&mut profile_ws, &push, aci, &pk, &phone).await
                            {
                                let _ = persist::save_profile(&to_stored_profile(&p));
                                *shared.profile.borrow_mut() = Some(p);
                                shared.push_event(Event::ProfileUpdated);
                            }
                        }
                    }
                }
                // On-demand groups refresh (sync_groups()).
                if shared.resync_groups.replace(false) {
                    spawn_group_fetch(shared, &push, aci, pni);
                }
            },
        }
    }
}

// ---- helpers ---------------------------------------------------------------

/// Download + decrypt the contacts-sync blob into the in-memory list + /state,
/// and emit a `contacts-updated` event. The sync is the full list, so it
/// replaces what we had.
async fn apply_contacts(
    shared: &Rc<Shared>,
    receiver: &mut MessageReceiver,
    blob: &sync_message::Contacts,
) -> Result<(), String> {
    let iter = receiver
        .retrieve_contacts(blob)
        .await
        .map_err(|e| format!("retrieve: {e}"))?;

    let mut list = Vec::new();
    let mut stored = Vec::new();
    for r in iter {
        let c = match r {
            Ok(c) => c,
            Err(_) => continue, // skip a malformed entry, keep the rest
        };
        let id = c.uuid.to_string();
        let phone = c.phone_number.as_ref().map(|p| p.to_string());
        let avatar = c.avatar.map(|a| a.reader.to_vec());
        list.push(Contact {
            id: id.clone(),
            name: c.name.clone(),
            phone: phone.clone(),
            inbox_position: c.inbox_position,
            avatar: avatar.clone(),
        });
        stored.push(persist::StoredContact {
            id,
            name: c.name,
            phone,
            inbox_position: c.inbox_position,
            avatar_b64: avatar.map(|b| b64().encode(b)),
        });
    }

    let n = list.len() as u32;
    *shared.contacts.borrow_mut() = list;
    let _ = persist::save_contacts(&stored);
    shared.push_event(Event::ContactsUpdated(n));
    Ok(())
}

/// Spawn a detached groups fetch if we have a master key (post-task-69 link);
/// otherwise skip silently (the UI shows 0 groups until a re-link captures it).
fn spawn_group_fetch(shared: &Rc<Shared>, push: &PushService, aci: Uuid, pni: Uuid) {
    let Some(mk) = shared.master_key.borrow().clone() else {
        return;
    };
    let shared = shared.clone();
    let push = push.clone();
    wart_step_executor::spawn(async move {
        if let Err(e) = fetch_groups(&shared, &mk, aci, pni, push).await {
            shared.set_state(format!("groups: {e}"));
        }
    })
    .detach();
}

/// Fetch the Groups v2 list via the storage service: derive the storage key from
/// the master key, read the encrypted manifest, pull the group-v2 records (each a
/// group master key), fetch + decrypt each group, resolve members against the
/// contacts, and persist to /state/groups.json.
async fn fetch_groups(
    shared: &Rc<Shared>,
    master_key: &[u8],
    aci: Uuid,
    pni: Uuid,
    push: PushService,
) -> Result<(), String> {
    let mk = MasterKey::from_slice(master_key)
        .map_err(|e| format!("master key: {e}"))?;
    let storage_key = StorageServiceKey::from_master_key(&mk);
    let mut storage = StorageService::new(push.clone(), storage_key)
        .await
        .map_err(|e| format!("storage auth: {e:?}"))?;
    let manifest = storage
        .manifest()
        .await
        .map_err(|e| format!("manifest: {e:?}"))?;

    let group_keys: Vec<Vec<u8>> = manifest
        .identifiers
        .iter()
        .filter(|id| {
            id.r#type == manifest_record::identifier::Type::Groupv2 as i32
        })
        .map(|id| id.raw.clone())
        .collect();

    let store_and_emit =
        |shared: &Rc<Shared>, out: Vec<Group>, routes: HashMap<String, GroupRoute>| {
            let n = out.len() as u32;
            let stored: Vec<persist::StoredGroup> = out
                .iter()
                .map(|g| {
                    let route = routes.get(&g.id);
                    persist::StoredGroup {
                        id: g.id.clone(),
                        title: g.title.clone(),
                        members: g.members.clone(),
                        avatar_b64: g.avatar.as_ref().map(|b| b64().encode(b)),
                        member_ids: route
                            .map(|r| r.members.iter().map(|u| u.to_string()).collect())
                            .unwrap_or_default(),
                        revision: route.map(|r| r.revision).unwrap_or(0),
                    }
                })
                .collect();
            *shared.groups.borrow_mut() = out;
            *shared.group_routes.borrow_mut() = routes;
            let _ = persist::save_groups(&stored);
            shared.push_event(Event::GroupsUpdated(n));
        };

    if group_keys.is_empty() {
        store_and_emit(shared, Vec::new(), HashMap::new());
        return Ok(());
    }

    let record_ikm = manifest.record_ikm.clone();
    let ikm: Option<&[u8]> =
        if record_ikm.is_empty() { None } else { Some(&record_ikm) };
    let records = storage
        .read_items(group_keys, ikm)
        .await
        .map_err(|e| format!("read items: {e:?}"))?;

    let server_public_params = ServiceConfiguration::from(SignalServers::Production)
        .zkgroup_server_public_params;
    let unidentified_ws = push
        .clone()
        .ws::<Unidentified>("/v1/websocket/", "/v1/keepalive", &[], None)
        .await
        .map_err(|e| format!("unidentified ws: {e}"))?;
    let mut gm = GroupsManager::new(
        ServiceIds { aci, pni },
        push.clone(),
        unidentified_ws,
        InMemoryCredentialsCache::default(),
        server_public_params,
    );
    let mut csprng = seed_rng();

    let mut out: Vec<Group> = Vec::new();
    let mut routes: HashMap<String, GroupRoute> = HashMap::new();
    for rec in records {
        let Some(storage_record::Record::GroupV2(gv2)) = rec.record else {
            continue;
        };
        let gmk = gv2.master_key;
        let enc = match gm.fetch_encrypted_group(&mut csprng, &gmk).await {
            Ok(g) => g,
            Err(_) => continue, // skip a group we can't fetch (e.g. left it)
        };
        let group = match decrypt_group(&gmk, enc) {
            Ok(g) => g,
            Err(_) => continue,
        };
        let members: Vec<String> = group
            .members
            .iter()
            .map(|m| {
                let mid = Uuid::from(m.aci);
                if mid == aci {
                    return "You".to_string();
                }
                let id = mid.to_string();
                shared
                    .contacts
                    .borrow()
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| c.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or(id)
            })
            .collect();
        // Group avatar: a CDN path encrypted with the group's secret params.
        // Fetch + decrypt it (best-effort — skip on any error).
        let avatar: Option<Vec<u8>> = if group.avatar.is_empty() {
            None
        } else {
            match <[u8; 32]>::try_from(gmk.as_slice()) {
                Ok(arr) => {
                    let gsp = GroupSecretParams::derive_from_master_key(
                        GroupMasterKey::new(arr),
                    );
                    gm.retrieve_avatar(&group.avatar, gsp).await.ok().flatten()
                }
                Err(_) => None,
            }
        };
        let id = b64().encode(&gmk);
        // Routing table for sends: raw member ACIs + the group's revision.
        routes.insert(
            id.clone(),
            GroupRoute {
                members: group.members.iter().map(|m| Uuid::from(m.aci)).collect(),
                revision: group.version,
            },
        );
        out.push(Group {
            id,
            title: group.title,
            members,
            avatar,
        });
    }

    store_and_emit(shared, out, routes);
    Ok(())
}

/// Pull `(body, outgoing, thread_override)` out of an incoming content body.
/// `thread_override` is `Some` when the message itself names its conversation:
/// a group (master key b64) for group messages, or the destination for a
/// sync-sent (my own message from another device). `None` ⇒ the caller defaults
/// the thread to the 1:1 peer (the envelope sender).
fn extract(body: &ContentBody) -> (Option<String>, bool, Option<String>, Option<u64>) {
    match body {
        ContentBody::DataMessage(dm) => (dm.body.clone(), false, group_thread(dm), dm.timestamp),
        ContentBody::SynchronizeMessage(sm) => {
            let sent = sm.sent.as_ref();
            let msg = sent.and_then(|s| s.message.as_ref());
            let thread = msg.and_then(group_thread).or_else(|| {
                sent.and_then(|s| s.destination_service_id.clone())
                    .map(|d| normalize_service_id(&d))
            });
            // The original send timestamp (so reactions match + time is the send
            // time), from the inner message or the sync-sent envelope.
            let wire_ts = msg.and_then(|m| m.timestamp).or_else(|| sent.and_then(|s| s.timestamp));
            (msg.and_then(|m| m.body.clone()), true, thread, wire_ts)
        },
        _ => (None, false, None, None),
    }
}

/// The attachment pointers on an incoming content body (direct or sync-sent).
fn attachments_of(body: &ContentBody) -> &[AttachmentPointer] {
    match body {
        ContentBody::DataMessage(dm) => &dm.attachments,
        ContentBody::SynchronizeMessage(sm) => sm
            .sent
            .as_ref()
            .and_then(|s| s.message.as_ref())
            .map(|m| m.attachments.as_slice())
            .unwrap_or(&[]),
        _ => &[],
    }
}

/// Download + decrypt one attachment from the CDN into its plaintext bytes (the
/// key material decrypts AES-CBC + strips the MAC; truncate to the real `size`).
/// Returns `None` on any failure — a missing media is non-fatal.
async fn fetch_attachment(push: &PushService, ptr: &AttachmentPointer) -> Option<Attachment> {
    use futures::io::AsyncReadExt;
    let mut push = push.clone();
    let mut stream = push.get_attachment(ptr).await.ok()?;
    let mut ciphertext = Vec::new();
    stream.read_to_end(&mut ciphertext).await.ok()?;
    let key_material = ptr.key();
    if key_material.len() != 64 {
        return None;
    }
    let mut key = [0u8; 64];
    key.copy_from_slice(key_material);
    decrypt_in_place(key, &mut ciphertext).ok()?;
    if let Some(sz) = ptr.size {
        ciphertext.truncate(sz as usize);
    }
    Some(Attachment {
        content_type: ptr.content_type.clone().unwrap_or_default(),
        data: ciphertext,
        width: ptr.width.unwrap_or(0),
        height: ptr.height.unwrap_or(0),
    })
}

/// The emoji reaction carried by an incoming content body, if any — either a
/// direct `DataMessage` reaction (a peer reacted to us) or a sync-sent one (we
/// reacted from another device).
fn reaction_of(body: &ContentBody) -> Option<&data_message::Reaction> {
    match body {
        ContentBody::DataMessage(dm) => dm.reaction.as_ref(),
        ContentBody::SynchronizeMessage(sm) => {
            sm.sent.as_ref()?.message.as_ref()?.reaction.as_ref()
        },
        _ => None,
    }
}

/// A group thread id (master key b64) if this DataMessage targets a group.
fn group_thread(dm: &DataMessage) -> Option<String> {
    dm.group_v2
        .as_ref()?
        .master_key
        .as_ref()
        .map(|mk| b64().encode(mk))
}

/// `"PNI:uuid"` / `"uuid"` → the bare lowercase uuid (matches contact ids).
fn normalize_service_id(s: &str) -> String {
    s.rsplit(':').next().unwrap_or(s).to_lowercase()
}

fn to_stored(m: &Message) -> persist::StoredMessage {
    // Persist each attachment's bytes to a file and record its metadata; the
    // message log stays small (just references).
    let attachments = m
        .attachments
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let ext = a.content_type.rsplit('/').next().filter(|e| !e.is_empty()).unwrap_or("bin");
            let file = format!("{}_{}.{ext}", m.ts, i);
            persist::save_attachment(&file, &a.data);
            persist::StoredAttachment {
                content_type: a.content_type.clone(),
                width: a.width,
                height: a.height,
                file,
            }
        })
        .collect();
    persist::StoredMessage {
        from: m.sender.clone(),
        text: m.text.clone(),
        ts: m.ts,
        outgoing: m.outgoing,
        thread: m.thread.clone(),
        attachments,
    }
}

/// Persisted profile → WIT record (avatar b64 → bytes).
fn to_wit_profile(p: persist::StoredProfile) -> Profile {
    Profile {
        given_name: p.given_name,
        family_name: p.family_name,
        about: p.about,
        about_emoji: p.about_emoji,
        phone: p.phone,
        avatar: p.avatar_b64.and_then(|b| b64().decode(b).ok()),
    }
}

/// WIT profile → persisted form (avatar bytes → b64).
fn to_stored_profile(p: &Profile) -> persist::StoredProfile {
    persist::StoredProfile {
        given_name: p.given_name.clone(),
        family_name: p.family_name.clone(),
        about: p.about.clone(),
        about_emoji: p.about_emoji.clone(),
        phone: p.phone.clone(),
        avatar_b64: p.avatar.as_ref().map(|b| b64().encode(b)),
    }
}

/// Fetch + decrypt our own Signal profile (name/about/avatar) over the identified
/// websocket, using our profile key. `None` on any failure (non-fatal — the UI
/// keeps whatever it had). The phone comes from the account, not the profile.
async fn fetch_own_profile(
    id_ws: &mut libsignal_service::websocket::SignalWebSocket<
        libsignal_service::websocket::Identified,
    >,
    push: &PushService,
    aci: Uuid,
    profile_key_bytes: &[u8],
    phone: &str,
) -> Option<Profile> {
    use futures::io::AsyncReadExt;
    let pk: [u8; 32] = profile_key_bytes.try_into().ok()?;
    let address = Aci::from(aci);
    let encrypted = id_ws
        .retrieve_profile_by_id(address, Some(ProfileKey::create(pk)))
        .await
        .ok()?;
    let avatar_path = encrypted.avatar.clone();
    let cipher = ProfileCipher::new(ProfileKey::create(pk));
    let decrypted = cipher.decrypt(encrypted).ok()?;
    // Avatar: a CDN path, fetched over an UNIDENTIFIED ws (plain CDN GET) and
    // decrypted with the same profile key.
    let avatar = match avatar_path {
        Some(path) if !path.is_empty() => {
            match push
                .clone()
                .ws::<Unidentified>("/v1/websocket/", "/v1/keepalive", &[], None)
                .await
            {
                Ok(mut un_ws) => match un_ws.retrieve_profile_avatar(&path).await {
                    Ok(mut stream) => {
                        let mut buf = Vec::new();
                        stream.read_to_end(&mut buf).await.ok()?;
                        cipher.decrypt_avatar(&buf).ok()
                    },
                    Err(_) => None,
                },
                Err(_) => None,
            }
        },
        _ => None,
    };
    let (given_name, family_name) = decrypted
        .name
        .map(|n| (n.given_name, n.family_name.unwrap_or_default()))
        .unwrap_or_default();
    Some(Profile {
        given_name,
        family_name,
        about: decrypted.about.unwrap_or_default(),
        about_emoji: decrypted.about_emoji.unwrap_or_default(),
        phone: phone.to_string(),
        avatar,
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn seed_rng() -> rand_chacha::ChaCha20Rng {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).expect("wasi getrandom");
    rand_chacha::ChaCha20Rng::from_seed(seed)
}

fn gen_password() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("wasi getrandom");
    b64().encode(bytes)
}
