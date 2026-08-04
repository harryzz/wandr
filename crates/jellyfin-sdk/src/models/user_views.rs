use serde::{Deserialize, Serialize};

/// Special view option dto.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SpecialViewOption {
    /// View option name.
    pub name: Option<String>,
    /// View option id.
    pub id: Option<String>,
}
