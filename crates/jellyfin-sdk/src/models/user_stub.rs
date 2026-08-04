use serde::{Deserialize, Serialize};

/// A minimal user DTO subset.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserStub {
    /// The user id.
    pub id: Option<uuid::Uuid>,
    /// The user name.
    pub name: Option<String>,
}
