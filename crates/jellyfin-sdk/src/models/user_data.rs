use serde::{Deserialize, Serialize};

/// Per-user data for a specific item.
///
/// OpenAPI: `UserItemDataDto`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserItemData {
    /// Gets or sets the rating.
    pub rating: Option<f64>,
    /// Gets or sets the played percentage.
    pub played_percentage: Option<f64>,
    /// Gets or sets the unplayed item count.
    pub unplayed_item_count: Option<i32>,
    /// Gets or sets the playback position ticks (100ns).
    pub playback_position_ticks: Option<i64>,
    /// Gets or sets the play count.
    pub play_count: Option<i32>,
    /// Gets or sets a value indicating whether this instance is favorite.
    pub is_favorite: Option<bool>,
    /// Gets or sets whether this is liked.
    pub likes: Option<bool>,
    /// Gets or sets the last played date.
    pub last_played_date: Option<String>,
    /// Gets or sets a value indicating whether this is played.
    pub played: Option<bool>,
    /// Gets or sets the key.
    pub key: Option<String>,
    /// Gets or sets the item identifier.
    pub item_id: Option<uuid::Uuid>,
}

/// Update payload for per-user item data.
///
/// OpenAPI: `UpdateUserItemDataDto`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateUserItemData {
    /// Gets or sets the rating.
    pub rating: Option<f64>,
    /// Gets or sets the played percentage.
    pub played_percentage: Option<f64>,
    /// Gets or sets the unplayed item count.
    pub unplayed_item_count: Option<i32>,
    /// Gets or sets the playback position ticks (100ns).
    pub playback_position_ticks: Option<i64>,
    /// Gets or sets the play count.
    pub play_count: Option<i32>,
    /// Gets or sets a value indicating whether this instance is favorite.
    pub is_favorite: Option<bool>,
    /// Gets or sets whether this is liked.
    pub likes: Option<bool>,
    /// Gets or sets the last played date.
    pub last_played_date: Option<String>,
    /// Gets or sets a value indicating whether this is played.
    pub played: Option<bool>,
    /// Gets or sets the key.
    pub key: Option<String>,
    /// Gets or sets the item identifier.
    pub item_id: Option<String>,
}

impl UpdateUserItemData {
    /// Creates an empty update.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets played flag.
    pub fn played(mut self, played: bool) -> Self {
        self.played = Some(played);
        self
    }

    /// Sets favorite flag.
    pub fn is_favorite(mut self, is_favorite: bool) -> Self {
        self.is_favorite = Some(is_favorite);
        self
    }

    /// Sets like flag.
    pub fn likes(mut self, likes: bool) -> Self {
        self.likes = Some(likes);
        self
    }

    /// Sets playback position ticks (100ns).
    pub fn playback_position_ticks(mut self, ticks: i64) -> Self {
        self.playback_position_ticks = Some(ticks);
        self
    }

    /// Sets played percentage.
    pub fn played_percentage(mut self, percent: f64) -> Self {
        self.played_percentage = Some(percent);
        self
    }

    /// Sets user rating.
    pub fn rating(mut self, rating: f64) -> Self {
        self.rating = Some(rating);
        self
    }

    /// Sets play count.
    pub fn play_count(mut self, play_count: i32) -> Self {
        self.play_count = Some(play_count);
        self
    }

    /// Sets last played date (ISO 8601).
    pub fn last_played_date(mut self, value: impl Into<String>) -> Self {
        self.last_played_date = Some(value.into());
        self
    }
}
