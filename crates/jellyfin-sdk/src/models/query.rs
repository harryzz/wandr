use serde::{Deserialize, Serialize};

/// A common Jellyfin query result container.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[serde(bound(deserialize = "T: Deserialize<'de>", serialize = "T: Serialize"))]
pub struct QueryResult<T> {
    /// Returned items.
    #[serde(default)]
    pub items: Vec<T>,
    /// Total number of records available (if provided by the endpoint).
    pub total_record_count: Option<u32>,
    /// Index of the first record in `items` (if provided by the endpoint).
    pub start_index: Option<u32>,
}
