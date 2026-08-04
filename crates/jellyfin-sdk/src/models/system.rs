use serde::{Deserialize, Serialize};

/// Public information about a Jellyfin server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PublicSystemInfo {
    /// The local address of the server.
    pub local_address: Option<String>,
    /// The configured server name.
    pub server_name: Option<String>,
    /// The server version.
    pub version: Option<String>,
    /// The product name.
    pub product_name: Option<String>,
    /// The operating system (deprecated by Jellyfin).
    pub operating_system: Option<String>,
    /// The server id.
    pub id: Option<String>,
    /// Whether the startup wizard is completed.
    pub startup_wizard_completed: Option<bool>,
}
