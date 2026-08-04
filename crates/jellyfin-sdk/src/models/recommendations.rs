use serde::{Deserialize, Serialize};

use crate::models::BaseItemDetailStub;

/// The reason/category for a recommendation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum RecommendationType {
    /// Similar to recently played items.
    SimilarToRecentlyPlayed,
    /// Similar to a liked item.
    SimilarToLikedItem,
    /// Shares a director with recently played items.
    HasDirectorFromRecentlyPlayed,
    /// Shares an actor with recently played items.
    HasActorFromRecentlyPlayed,
    /// Has a liked director.
    HasLikedDirector,
    /// Has a liked actor.
    HasLikedActor,
}

/// A minimal subset of `RecommendationDto`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RecommendationStub {
    /// Recommended items in this category.
    #[serde(default)]
    pub items: Vec<BaseItemDetailStub>,
    /// Recommendation category.
    pub recommendation_type: Option<RecommendationType>,
    /// Baseline item name, if applicable.
    pub baseline_item_name: Option<String>,
    /// Category id.
    pub category_id: Option<uuid::Uuid>,
}
