use serde::{Deserialize, Serialize};

use crate::models::MediaType;

/// Playlist user permissions.
///
/// OpenAPI: `PlaylistUserPermissions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaylistUserPermissions {
    /// The user id.
    pub user_id: uuid::Uuid,
    /// Whether the user can edit the playlist.
    pub can_edit: bool,
}

/// Create playlist request body.
///
/// OpenAPI: `CreatePlaylistDto`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreatePlaylist {
    /// The name of the new playlist.
    pub name: String,
    /// Item ids to add to the playlist.
    #[serde(default)]
    pub ids: Vec<uuid::Uuid>,
    /// The user id.
    pub user_id: Option<uuid::Uuid>,
    /// The media type.
    pub media_type: Option<MediaType>,
    /// Playlist users.
    #[serde(default)]
    pub users: Vec<PlaylistUserPermissions>,
    /// Whether the playlist is public.
    #[serde(default)]
    pub is_public: bool,
}

impl CreatePlaylist {
    /// Creates a new playlist request.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ids: Vec::new(),
            user_id: None,
            media_type: None,
            users: Vec::new(),
            is_public: false,
        }
    }

    /// Adds an item id.
    pub fn id(mut self, id: uuid::Uuid) -> Self {
        self.ids.push(id);
        self
    }

    /// Sets the user id.
    pub fn user_id(mut self, user_id: uuid::Uuid) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// Sets the media type.
    pub fn media_type(mut self, media_type: MediaType) -> Self {
        self.media_type = Some(media_type);
        self
    }

    /// Sets whether the playlist is public.
    pub fn is_public(mut self, is_public: bool) -> Self {
        self.is_public = is_public;
        self
    }

    /// Adds a user permission entry.
    pub fn user(mut self, user: PlaylistUserPermissions) -> Self {
        self.users.push(user);
        self
    }
}

/// Playlist creation result.
///
/// OpenAPI: `PlaylistCreationResult`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaylistCreationResult {
    /// Created playlist id.
    pub id: Option<String>,
}

/// Playlist DTO.
///
/// OpenAPI: `PlaylistDto`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaylistDto {
    /// Whether the playlist is publicly readable.
    pub open_access: Option<bool>,
    /// Share permissions.
    #[serde(default)]
    pub shares: Vec<PlaylistUserPermissions>,
    /// Playlist item ids.
    #[serde(default)]
    pub item_ids: Vec<uuid::Uuid>,
}

/// Update playlist request body.
///
/// OpenAPI: `UpdatePlaylistDto`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdatePlaylist {
    /// New playlist name.
    pub name: Option<String>,
    /// Playlist item ids.
    pub ids: Option<Vec<uuid::Uuid>>,
    /// Playlist users.
    pub users: Option<Vec<PlaylistUserPermissions>>,
    /// Whether the playlist is public.
    pub is_public: Option<bool>,
}

impl UpdatePlaylist {
    /// Creates an empty update request.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the playlist name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets whether the playlist is public.
    pub fn is_public(mut self, is_public: bool) -> Self {
        self.is_public = Some(is_public);
        self
    }
}

/// Update a playlist user request body.
///
/// OpenAPI: `UpdatePlaylistUserDto`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdatePlaylistUser {
    /// Whether the user can edit the playlist.
    pub can_edit: Option<bool>,
}

impl UpdatePlaylistUser {
    /// Creates an empty update request.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets whether the user can edit the playlist.
    pub fn can_edit(mut self, can_edit: bool) -> Self {
        self.can_edit = Some(can_edit);
        self
    }
}
