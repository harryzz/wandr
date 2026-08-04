use serde::{Deserialize, Serialize};

/// Metadata refresh mode.
///
/// OpenAPI: `MetadataRefreshMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum MetadataRefreshMode {
    /// No refresh.
    None,
    /// Validate only.
    ValidationOnly,
    /// Default refresh.
    Default,
    /// Full refresh.
    FullRefresh,
}

impl MetadataRefreshMode {
    /// Returns the wire representation used by Jellyfin.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::ValidationOnly => "ValidationOnly",
            Self::Default => "Default",
            Self::FullRefresh => "FullRefresh",
        }
    }
}

impl std::fmt::Display for MetadataRefreshMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
