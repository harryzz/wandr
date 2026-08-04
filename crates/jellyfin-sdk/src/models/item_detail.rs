use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::models::{BaseItemKind, NameGuidPair};

/// A minimal-but-useful subset of `BaseItemDto` for item detail screens.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BaseItemDetailStub {
    /// The item id.
    pub id: Option<uuid::Uuid>,
    /// The display name.
    pub name: Option<String>,
    /// The item kind.
    #[serde(rename = "Type")]
    pub kind: Option<BaseItemKind>,
    /// The parent item id.
    pub parent_id: Option<uuid::Uuid>,
    /// Whether this item is a folder.
    pub is_folder: Option<bool>,

    /// Item overview/summary.
    pub overview: Option<String>,
    /// Taglines.
    pub taglines: Option<Vec<String>>,
    /// Production year.
    pub production_year: Option<i32>,
    /// Community rating.
    pub community_rating: Option<f32>,
    /// Run time in ticks (100ns).
    pub run_time_ticks: Option<i64>,
    /// Official rating (PG, TV-MA, etc).
    pub official_rating: Option<String>,
    /// Studios.
    pub studios: Option<Vec<NameGuidPair>>,
    /// Genres.
    pub genres: Option<Vec<String>>,
    /// Tags.
    pub tags: Option<Vec<String>>,
    /// External provider ids (e.g. IMDb, TMDb).
    pub provider_ids: Option<BTreeMap<String, Option<String>>>,
}
