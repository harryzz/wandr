use serde::{Deserialize, Serialize};

/// Request body for `POST /Users/AuthenticateByName`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AuthenticateUserByName {
    /// The username.
    pub username: Option<String>,
    /// The plain text password.
    #[serde(rename = "Pw")]
    pub pw: Option<String>,
}

/// Response body for `POST /Users/AuthenticateByName`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AuthenticationResult {
    /// User object (not fully modeled yet).
    pub user: Option<serde_json::Value>,
    /// Session info object (not fully modeled yet).
    pub session_info: Option<serde_json::Value>,
    /// The access token.
    pub access_token: Option<String>,
    /// The server id.
    pub server_id: Option<String>,
}
