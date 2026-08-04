use serde::{Deserialize, Serialize};

use crate::models::ImageType;

/// Image info for an item.
///
/// OpenAPI: `ImageInfo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ImageInfo {
    /// The type of the image.
    pub image_type: Option<ImageType>,
    /// The index of the image.
    pub image_index: Option<i32>,
    /// The image tag.
    pub image_tag: Option<String>,
    /// The filesystem path.
    pub path: Option<String>,
    /// BlurHash.
    pub blur_hash: Option<String>,
    /// Height in pixels.
    pub height: Option<i32>,
    /// Width in pixels.
    pub width: Option<i32>,
    /// Size in bytes.
    pub size: Option<i64>,
}
