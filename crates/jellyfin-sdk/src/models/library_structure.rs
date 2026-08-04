use serde::{Deserialize, Serialize};

/// Collection type options for virtual folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollectionType {
    /// Movies library.
    Movies,
    /// TV shows library.
    Tvshows,
    /// Music library.
    Music,
    /// Music videos library.
    Musicvideos,
    /// Home videos library.
    Homevideos,
    /// Box sets library.
    Boxsets,
    /// Books library.
    Books,
    /// Mixed library.
    Mixed,
}

impl CollectionType {
    /// Returns the wire representation used by Jellyfin.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Movies => "movies",
            Self::Tvshows => "tvshows",
            Self::Music => "music",
            Self::Musicvideos => "musicvideos",
            Self::Homevideos => "homevideos",
            Self::Boxsets => "boxsets",
            Self::Books => "books",
            Self::Mixed => "mixed",
        }
    }
}

impl std::fmt::Display for CollectionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Add virtual folder request body.
///
/// OpenAPI: `AddVirtualFolderDto`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AddVirtualFolderBody {
    /// Library options for the new folder.
    pub library_options: Option<serde_json::Value>,
}

/// Update library options request body.
///
/// OpenAPI: `UpdateLibraryOptionsDto`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateLibraryOptionsRequest {
    /// The library item id.
    pub id: uuid::Uuid,
    /// Library options payload.
    pub library_options: Option<serde_json::Value>,
}

/// Media path information.
///
/// OpenAPI: `MediaPathInfo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaPathInfo {
    /// The filesystem path.
    pub path: String,
}

/// Media path DTO.
///
/// OpenAPI: `MediaPathDto`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaPath {
    /// The name of the library.
    pub name: String,
    /// The path to add.
    pub path: Option<String>,
    /// The path info.
    pub path_info: Option<MediaPathInfo>,
}

/// Update media path request body.
///
/// OpenAPI: `UpdateMediaPathRequestDto`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateMediaPathRequest {
    /// The library name.
    pub name: String,
    /// Library folder path information.
    pub path_info: MediaPathInfo,
}

/// Virtual folder information.
///
/// OpenAPI: `VirtualFolderInfo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct VirtualFolderInfo {
    /// The folder name.
    pub name: Option<String>,
    /// Locations included by this folder.
    pub locations: Option<Vec<String>>,
    /// The collection type.
    pub collection_type: Option<CollectionType>,
    /// Library options payload.
    pub library_options: Option<serde_json::Value>,
    /// Item id for this virtual folder.
    pub item_id: Option<String>,
    /// Primary image item id.
    pub primary_image_item_id: Option<String>,
    /// Refresh progress (0..100).
    pub refresh_progress: Option<f64>,
    /// Refresh status.
    pub refresh_status: Option<String>,
}
