use serde::{Deserialize, Serialize};

use crate::models::BaseItemKind;

/// The base media type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum MediaType {
    /// Unknown media type.
    Unknown,
    /// Video media.
    Video,
    /// Audio media.
    Audio,
    /// Photo media.
    Photo,
    /// Book media.
    Book,
}

impl MediaType {
    /// Returns the wire representation used by Jellyfin.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Photo => "Photo",
            Self::Book => "Book",
        }
    }
}

impl std::fmt::Display for MediaType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A minimal subset of Jellyfin `SearchHint`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SearchHintStub {
    /// The item id.
    pub id: Option<uuid::Uuid>,
    /// The display name.
    pub name: Option<String>,
    /// The item kind.
    #[serde(rename = "Type")]
    pub kind: Option<BaseItemKind>,
    /// The matched term.
    pub matched_term: Option<String>,
    /// Production year.
    pub production_year: Option<i32>,
    /// Whether this item is a folder.
    pub is_folder: Option<bool>,
    /// Image tag for primary image.
    pub primary_image_tag: Option<String>,
}

/// A minimal subset of Jellyfin `SearchHintResult`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SearchHintResultStub {
    /// Search hints.
    #[serde(default)]
    pub search_hints: Vec<SearchHintStub>,
    /// Total record count.
    pub total_record_count: Option<u32>,
}
