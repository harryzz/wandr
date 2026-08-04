use serde::{Deserialize, Serialize};

/// API key entry.
///
/// OpenAPI: `AuthenticationInfo`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AuthenticationInfo {
    /// Identifier.
    pub id: Option<i64>,
    /// API key (access token).
    pub access_token: Option<String>,
    /// Device identifier.
    pub device_id: Option<String>,
    /// Application name.
    pub app_name: Option<String>,
    /// Application version.
    pub app_version: Option<String>,
    /// Device name.
    pub device_name: Option<String>,
    /// User identifier.
    pub user_id: Option<uuid::Uuid>,
    /// Whether this key is active.
    pub is_active: Option<bool>,
    /// Date created (ISO 8601).
    pub date_created: Option<String>,
    /// Date revoked (ISO 8601).
    pub date_revoked: Option<String>,
    /// Date of last activity (ISO 8601).
    pub date_last_activity: Option<String>,
    /// User name.
    pub user_name: Option<String>,
}
