use serde::{Deserialize, Serialize};

use crate::models::NameGuidPair;

/// A minimal subset of Jellyfin `QueryFilters`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueryFilters {
    /// Genre facets.
    pub genres: Option<Vec<NameGuidPair>>,
    /// Tag facets.
    pub tags: Option<Vec<String>>,
}
