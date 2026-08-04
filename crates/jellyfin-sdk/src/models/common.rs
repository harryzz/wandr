use serde::{Deserialize, Serialize};

/// A common `(Name, Id)` pair used across Jellyfin DTOs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NameGuidPair {
    /// Display name.
    pub name: Option<String>,
    /// Identifier.
    pub id: Option<uuid::Uuid>,
}
