use serde::{Deserialize, Serialize};

/// Plugin load status.
///
/// OpenAPI: `PluginStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PluginStatus {
    /// Plugin is active.
    Active,
    /// Plugin requires a server restart.
    Restart,
    /// Plugin is deleted.
    Deleted,
    /// Plugin was superseded.
    Superseded,
    /// Plugin was superseded (legacy misspelling).
    Superceded,
    /// Plugin is malfunctioned.
    Malfunctioned,
    /// Plugin is not supported.
    NotSupported,
    /// Plugin is disabled.
    Disabled,
}

/// Installed plugin information.
///
/// OpenAPI: `PluginInfo`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PluginInfo {
    /// Name.
    pub name: Option<String>,
    /// Version string.
    pub version: Option<String>,
    /// Configuration file name.
    pub configuration_file_name: Option<String>,
    /// Description.
    pub description: Option<String>,
    /// Unique id.
    pub id: Option<uuid::Uuid>,
    /// Whether the plugin can be uninstalled.
    pub can_uninstall: Option<bool>,
    /// Whether the plugin has a valid image.
    pub has_image: Option<bool>,
    /// Status.
    pub status: Option<PluginStatus>,
}
