//! Pure-Rust file persistence over WASI fs (guest preopen `/state`). Simple by
//! design (testing, not production): rewrite the whole account/store snapshot on
//! change, append message history as JSON lines. Lets the device link once and
//! reconnect on restart instead of re-linking.

use serde::{Deserialize, Serialize};

const DIR: &str = "/state";
const ACCOUNT: &str = "/state/account.json";
const SNAPSHOT: &str = "/state/store.json";
const MESSAGES: &str = "/state/messages.jsonl";

/// Everything needed to re-authenticate without re-linking.
#[derive(Serialize, Deserialize)]
pub struct Account {
    pub aci: uuid::Uuid,
    pub pni: uuid::Uuid,
    pub number: String,
    pub password: String,
    pub device_id: u32,
    pub registration_id: u32,
    /// base64 of `IdentityKeyPair::serialize()` (the provisioned ACI identity).
    pub identity_b64: String,
    /// base64 of the account master key (from provisioning) — needed for the
    /// storage service + Groups v2 (task 69). `default` so older account.json
    /// (pre-groups) still loads; absence means a re-link is needed for groups.
    #[serde(default)]
    pub master_key_b64: Option<String>,
    /// base64 of our OWN profile key (32 bytes), captured from the provisioning
    /// message at link — it is ONLY available then, so missing it means a
    /// re-link. Needed to fetch + decrypt our own Signal profile (name/avatar/
    /// about). `default` so older account.json still loads.
    #[serde(default)]
    pub profile_key_b64: Option<String>,
}

const PROFILE: &str = "/state/profile.json";

/// Our own Signal profile (fetched via the profile service + decrypted with our
/// profile key). The full record is persisted for future use; the UI currently
/// shows name + avatar + phone.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct StoredProfile {
    pub given_name: String,
    pub family_name: String,
    pub about: String,
    pub about_emoji: String,
    /// E.164 phone number (from the account, not the profile).
    pub phone: String,
    /// base64 of the decrypted avatar image bytes, if the profile has one.
    pub avatar_b64: Option<String>,
}

pub fn load_profile() -> Option<StoredProfile> {
    std::fs::read(PROFILE).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

pub fn save_profile(p: &StoredProfile) -> std::io::Result<()> {
    ensure_dir();
    std::fs::write(PROFILE, serde_json::to_vec_pretty(p).unwrap())
}

#[derive(Serialize, Deserialize)]
pub struct StoredMessage {
    pub from: String,
    pub text: String,
    pub ts: u64,
    pub outgoing: bool,
    /// Conversation key (peer ACI / group master key b64). `default` so older
    /// messages.jsonl lines (pre-task-70) still load — they fall into thread "".
    #[serde(default)]
    pub thread: String,
    /// Media attachments — the bytes live in files under `/state/att/` (CDN
    /// blobs expire, so we keep them); this records each one's type/dims/file.
    #[serde(default)]
    pub attachments: Vec<StoredAttachment>,
    /// Call-history entry kind (`out-answered`/`in-missed`/…) when this row is a
    /// call log rather than a message. `default` None ⇒ a normal message.
    #[serde(default)]
    pub call: Option<String>,
}

/// A persisted attachment: metadata in the message log, bytes in `/state/att/`.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct StoredAttachment {
    pub content_type: String,
    pub width: u32,
    pub height: u32,
    /// File name under `/state/att/` holding the decrypted bytes.
    pub file: String,
}

const ATT_DIR: &str = "/state/att";

/// Write one attachment's decrypted bytes to `/state/att/<name>`.
pub fn save_attachment(name: &str, bytes: &[u8]) {
    let _ = std::fs::create_dir_all(ATT_DIR);
    let _ = std::fs::write(format!("{ATT_DIR}/{name}"), bytes);
}

/// Read an attachment's bytes back (e.g. on history preload).
pub fn load_attachment(name: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("{ATT_DIR}/{name}")).ok()
}

fn ensure_dir() {
    let _ = std::fs::create_dir_all(DIR);
}

pub fn load_account() -> Option<Account> {
    let bytes = std::fs::read(ACCOUNT).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn save_account(account: &Account) -> std::io::Result<()> {
    ensure_dir();
    std::fs::write(ACCOUNT, serde_json::to_vec_pretty(account).unwrap())
}

pub fn load_snapshot() -> Option<Vec<u8>> {
    std::fs::read(SNAPSHOT).ok()
}

pub fn save_snapshot(bytes: &[u8]) -> std::io::Result<()> {
    ensure_dir();
    std::fs::write(SNAPSHOT, bytes)
}

pub fn append_message(msg: &StoredMessage) -> std::io::Result<()> {
    use std::io::Write;
    ensure_dir();
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(MESSAGES)?;
    writeln!(f, "{}", serde_json::to_string(msg).unwrap())
}

const OUTBOX: &str = "/state/outbox.txt";

/// Drain the outbox: each non-empty line is a note-to-self to send. Deletes the
/// file so messages aren't resent on the next run.
pub fn take_outbox() -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(OUTBOX) else {
        return Vec::new();
    };
    let _ = std::fs::remove_file(OUTBOX);
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect()
}

pub fn load_messages() -> Vec<StoredMessage> {
    std::fs::read_to_string(MESSAGES)
        .map(|s| {
            s.lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        })
        .unwrap_or_default()
}

const STATUSES: &str = "/state/statuses.json";

/// Persisted delivery state for outgoing messages, keyed by wire ts → code
/// (0 sending, 1 sent, 2 delivered, 3 read). Kept out of the append-only message
/// log so a receipt can update one message without rewriting history.
pub fn load_statuses() -> std::collections::HashMap<u64, u8> {
    std::fs::read(STATUSES)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Record one message's delivery state (read-modify-write the small map).
pub fn set_status(ts: u64, code: u8) {
    let mut map = load_statuses();
    map.insert(ts, code);
    ensure_dir();
    if let Ok(bytes) = serde_json::to_vec(&map) {
        let _ = std::fs::write(STATUSES, bytes);
    }
}

const REACTIONS: &str = "/state/reactions.json";

/// Persisted emoji reactions, keyed by the target message's wire ts → (reactor
/// ACI → emoji). Like statuses, kept out of the append-only log so one reaction
/// updates without rewriting history.
pub fn load_reactions() -> std::collections::HashMap<u64, std::collections::HashMap<String, String>> {
    std::fs::read(REACTIONS)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Record one message's reactor→emoji map (read-modify-write). An empty map
/// removes the entry (all reactions cleared).
pub fn set_reactions(ts: u64, reactors: &std::collections::HashMap<String, String>) {
    let mut map = load_reactions();
    if reactors.is_empty() {
        map.remove(&ts);
    } else {
        map.insert(ts, reactors.clone());
    }
    ensure_dir();
    if let Ok(bytes) = serde_json::to_vec(&map) {
        let _ = std::fs::write(REACTIONS, bytes);
    }
}

const CONTACTS: &str = "/state/contacts.json";

/// A user from the primary device's contact list (Signal contacts-sync).
#[derive(Serialize, Deserialize, Clone)]
pub struct StoredContact {
    pub id: String, // ACI (account UUID)
    pub name: String,
    pub phone: Option<String>,
    pub inbox_position: u32,
    /// base64 of the encoded avatar image, if any. `default` so older
    /// contacts.json (no avatars) still loads.
    #[serde(default)]
    pub avatar_b64: Option<String>,
}

pub fn load_contacts() -> Vec<StoredContact> {
    std::fs::read(CONTACTS)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_contacts(contacts: &[StoredContact]) -> std::io::Result<()> {
    ensure_dir();
    std::fs::write(CONTACTS, serde_json::to_vec_pretty(contacts).unwrap())
}

const GROUPS: &str = "/state/groups.json";

/// A Signal group (Groups v2) fetched via the storage service.
#[derive(Serialize, Deserialize, Clone)]
pub struct StoredGroup {
    /// hex of the group master key — the stable group identity.
    pub id: String,
    pub title: String,
    /// Member display names (resolved against contacts where possible, else ACI).
    pub members: Vec<String>,
    /// base64 of the decrypted group avatar (JPEG), if any. `default` so older
    /// groups.json (no avatars) still loads.
    #[serde(default)]
    pub avatar_b64: Option<String>,
    /// Raw member ACI uuids — needed to address `send_message_to_group`.
    #[serde(default)]
    pub member_ids: Vec<String>,
    /// The group's current revision (for the outgoing `GroupContextV2`).
    #[serde(default)]
    pub revision: u32,
}

pub fn load_groups() -> Vec<StoredGroup> {
    std::fs::read(GROUPS)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_groups(groups: &[StoredGroup]) -> std::io::Result<()> {
    ensure_dir();
    std::fs::write(GROUPS, serde_json::to_vec_pretty(groups).unwrap())
}
