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
