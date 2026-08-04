//! First-run pairing + session persistence — the ONE thing the Jellyfin SDK doesn't
//! cover (Quick Connect isn't in Jellyfin's OpenAPI as a client op). Three tiny calls
//! over `wandr-reqwest`/`wasi:tls`; everything else (browse/playback/images/reporting)
//! goes through `jellyfin-sdk`. Once paired, the saved token drives the SDK directly.

use serde_json::{json, Value};
use url::Url;

/// Demo default; overridable by writing a bare URL to `/state/jellyfin/server`.
pub const DEFAULT_SERVER: &str = "https://movies.zaharievi.eu";

const STATE_DIR: &str = "/state/jellyfin";
const SESSION_PATH: &str = "/state/jellyfin/session.json";
const DEVICE_ID_PATH: &str = "/state/jellyfin/device_id";
const SERVER_OVERRIDE_PATH: &str = "/state/jellyfin/server";

/// Persisted session (`/state/jellyfin/session.json`). Jellyfin tokens are long-lived.
#[derive(Clone, Debug)]
pub struct Session {
    pub server_url: String,
    pub user_id: String,
    pub device_id: String,
    pub access_token: String,
}

impl Session {
    pub fn from_json(v: &Value) -> Option<Session> {
        Some(Session {
            server_url: v.get("server_url")?.as_str()?.to_string(),
            user_id: v.get("user_id")?.as_str()?.to_string(),
            device_id: v.get("device_id")?.as_str()?.to_string(),
            access_token: v.get("access_token")?.as_str()?.to_string(),
        })
    }
    pub fn to_json(&self) -> Value {
        json!({
            "server_url": self.server_url,
            "user_id": self.user_id,
            "device_id": self.device_id,
            "access_token": self.access_token,
        })
    }
}

fn ensure_state_dir() {
    let _ = std::fs::create_dir_all(STATE_DIR);
}
pub fn load_session() -> Option<Session> {
    let bytes = std::fs::read(SESSION_PATH).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    Session::from_json(&v)
}
pub fn save_session(s: &Session) -> std::io::Result<()> {
    ensure_state_dir();
    std::fs::write(SESSION_PATH, serde_json::to_vec_pretty(&s.to_json()).unwrap())
}
pub fn clear_session() {
    let _ = std::fs::remove_file(SESSION_PATH);
}
pub fn server_override() -> Option<String> {
    let s = std::fs::read_to_string(SERVER_OVERRIDE_PATH).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}
/// Stable per-install device id (from the wall clock once, then persisted).
pub fn load_or_make_device_id() -> String {
    if let Ok(s) = std::fs::read_to_string(DEVICE_ID_PATH) {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let id = format!("wandr-{nanos:032x}");
    ensure_state_dir();
    let _ = std::fs::write(DEVICE_ID_PATH, &id);
    id
}

fn url(base: &str, path: &str) -> Result<Url, String> {
    Url::parse(&format!("{}{}", base.trim_end_matches('/'), path)).map_err(|e| format!("bad url: {e}"))
}
fn auth_header(device_id: &str) -> String {
    format!("MediaBrowser Client=\"wandr\", Device=\"wandr\", DeviceId=\"{device_id}\", Version=\"0.1.0\"")
}

/// A pending Quick Connect pairing: the 6-digit `code` the user types into Jellyfin,
/// and the `secret` we poll/exchange with.
pub struct QuickConnect {
    pub secret: String,
    pub code: String,
}

pub async fn qc_enabled(client: &reqwest::Client, server: &str) -> bool {
    let Ok(u) = url(server, "/QuickConnect/Enabled") else { return false };
    match client.get(u).send().await {
        Ok(r) => r.text().await.map(|t| t.trim() == "true").unwrap_or(false),
        Err(_) => false,
    }
}

pub async fn qc_initiate(client: &reqwest::Client, server: &str, device_id: &str) -> Result<QuickConnect, String> {
    let v: Value = client
        .get(url(server, "/QuickConnect/Initiate")?)
        .header("Authorization", auth_header(device_id))
        .send().await.map_err(|e| format!("qc initiate: {e}"))?
        .json().await.map_err(|e| format!("qc initiate json: {e}"))?;
    Ok(QuickConnect {
        secret: v.get("Secret").and_then(|s| s.as_str()).ok_or("no Secret")?.to_string(),
        code: v.get("Code").and_then(|s| s.as_str()).ok_or("no Code")?.to_string(),
    })
}

pub async fn qc_poll(client: &reqwest::Client, server: &str, device_id: &str, secret: &str) -> Result<bool, String> {
    let v: Value = client
        .get(url(server, &format!("/QuickConnect/Connect?Secret={secret}"))?)
        .header("Authorization", auth_header(device_id))
        .send().await.map_err(|e| format!("qc poll: {e}"))?
        .json().await.map_err(|e| format!("qc poll json: {e}"))?;
    Ok(v.get("Authenticated").and_then(|b| b.as_bool()).unwrap_or(false))
}

/// Exchange an approved secret for (access_token, user_id).
pub async fn qc_exchange(client: &reqwest::Client, server: &str, device_id: &str, secret: &str) -> Result<(String, String), String> {
    let v: Value = client
        .request(reqwest::Method::POST, url(server, "/Users/AuthenticateWithQuickConnect")?)
        .header("Authorization", auth_header(device_id))
        .json(&json!({ "Secret": secret }))
        .send().await.map_err(|e| format!("qc exchange: {e}"))?
        .json().await.map_err(|e| format!("qc exchange json: {e}"))?;
    let token = v.get("AccessToken").and_then(|s| s.as_str()).ok_or("no AccessToken")?.to_string();
    let user_id = v.get("User").and_then(|u| u.get("Id")).and_then(|s| s.as_str()).ok_or("no User.Id")?.to_string();
    Ok((token, user_id))
}
