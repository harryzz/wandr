use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{
        BaseItemStub, CreatePlaylist, ImageType, ItemField, PlaylistCreationResult, PlaylistDto,
        PlaylistUserPermissions, QueryResult, UpdatePlaylist, UpdatePlaylistUser,
    },
};

/// Query parameters for `GET /Playlists/{playlistId}/Items`.
#[derive(Clone, Debug, Default)]
pub struct PlaylistItemsQuery {
    params: Vec<(String, String)>,
    start_index: Option<u32>,
    limit: Option<u32>,
    fields: Vec<ItemField>,
    enable_image_types: Vec<ImageType>,
}

impl PlaylistItemsQuery {
    /// Creates an empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// User id.
    pub fn user_id(mut self, user_id: uuid::Uuid) -> Self {
        self.params.push(("userId".to_owned(), user_id.to_string()));
        self
    }

    /// Sets `startIndex`.
    pub fn start_index(mut self, start_index: u32) -> Self {
        self.start_index = Some(start_index);
        self
    }

    /// Sets `limit`.
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Specifies additional fields of information to return.
    pub fn field(mut self, field: ItemField) -> Self {
        self.fields.push(field);
        self
    }

    /// Whether to include image information in output.
    pub fn enable_images(mut self, enable: bool) -> Self {
        self.params
            .push(("enableImages".to_owned(), enable.to_string()));
        self
    }

    /// Whether to include user data in output.
    pub fn enable_user_data(mut self, enable: bool) -> Self {
        self.params
            .push(("enableUserData".to_owned(), enable.to_string()));
        self
    }

    /// The max number of images to return, per image type.
    pub fn image_type_limit(mut self, limit: u32) -> Self {
        self.params
            .push(("imageTypeLimit".to_owned(), limit.to_string()));
        self
    }

    /// The image types to include in the output.
    pub fn enable_image_type(mut self, image_type: ImageType) -> Self {
        self.enable_image_types.push(image_type);
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }

    fn to_params(&self) -> Vec<(String, String)> {
        let mut q = self.params.clone();

        if let Some(v) = self.start_index {
            q.push(("startIndex".to_owned(), v.to_string()));
        }
        if let Some(v) = self.limit {
            q.push(("limit".to_owned(), v.to_string()));
        }

        push_joined(&mut q, "fields", self.fields.iter().map(|v| v.to_string()));
        push_joined(
            &mut q,
            "enableImageTypes",
            self.enable_image_types.iter().map(|v| v.to_string()),
        );

        q
    }
}

/// Playlist endpoints.
#[derive(Clone, Debug)]
pub struct PlaylistsApi {
    client: JellyfinClient,
}

impl PlaylistsApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Creates a new playlist.
    ///
    /// OpenAPI: `POST /Playlists` (`CreatePlaylist`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn create_playlist(&self, request: CreatePlaylist) -> Result<PlaylistCreationResult> {
        let req = self
            .client
            .request(Method::POST, "Playlists")?
            .json(&request);
        self.client.send_json(req).await
    }

    /// Gets a playlist.
    ///
    /// OpenAPI: `GET /Playlists/{playlistId}` (`GetPlaylist`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_playlist(&self, playlist_id: uuid::Uuid) -> Result<PlaylistDto> {
        let req = self
            .client
            .request(Method::GET, &format!("Playlists/{playlist_id}"))?;
        self.client.send_json(req).await
    }

    /// Updates a playlist.
    ///
    /// OpenAPI: `POST /Playlists/{playlistId}` (`UpdatePlaylist`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn update_playlist(
        &self,
        playlist_id: uuid::Uuid,
        request: UpdatePlaylist,
    ) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, &format!("Playlists/{playlist_id}"))?
            .json(&request);
        self.client.send_unit(req).await
    }

    /// Gets the original items of a playlist.
    ///
    /// OpenAPI: `GET /Playlists/{playlistId}/Items` (`GetPlaylistItems`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_playlist_items(
        &self,
        playlist_id: uuid::Uuid,
        query: PlaylistItemsQuery,
    ) -> Result<QueryResult<BaseItemStub>> {
        let req = self
            .client
            .request(Method::GET, &format!("Playlists/{playlist_id}/Items"))?
            .query(&query.to_params());
        self.client.send_json(req).await
    }

    /// Adds items to a playlist.
    ///
    /// OpenAPI: `POST /Playlists/{playlistId}/Items` (`AddItemToPlaylist`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn add_items_to_playlist(
        &self,
        playlist_id: uuid::Uuid,
        ids: Vec<uuid::Uuid>,
        user_id: Option<uuid::Uuid>,
    ) -> Result<()> {
        let mut params: Vec<(String, String)> = Vec::new();
        for id in ids {
            params.push(("ids".to_owned(), id.to_string()));
        }
        if let Some(user_id) = user_id {
            params.push(("userId".to_owned(), user_id.to_string()));
        }

        let req = self
            .client
            .request(Method::POST, &format!("Playlists/{playlist_id}/Items"))?
            .query(&params);
        self.client.send_unit(req).await
    }

    /// Removes items from a playlist.
    ///
    /// Note: this uses `entryIds` (playlist entry ids), not media item ids.
    ///
    /// OpenAPI: `DELETE /Playlists/{playlistId}/Items` (`RemoveItemFromPlaylist`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn remove_items_from_playlist(
        &self,
        playlist_id: uuid::Uuid,
        entry_ids: Vec<String>,
    ) -> Result<()> {
        let mut params: Vec<(String, String)> = Vec::new();
        for id in entry_ids {
            params.push(("entryIds".to_owned(), id));
        }

        let req = self
            .client
            .request(Method::DELETE, &format!("Playlists/{playlist_id}/Items"))?
            .query(&params);
        self.client.send_unit(req).await
    }

    /// Moves a playlist item.
    ///
    /// Note: the OpenAPI schema calls this `itemId`, but servers treat it as a playlist entry id.
    ///
    /// OpenAPI: `POST /Playlists/{playlistId}/Items/{itemId}/Move/{newIndex}` (`MoveItem`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn move_item(
        &self,
        playlist_id: uuid::Uuid,
        item_id: impl AsRef<str>,
        new_index: i32,
    ) -> Result<()> {
        let req = self.client.request(
            Method::POST,
            &format!(
                "Playlists/{playlist_id}/Items/{}/Move/{new_index}",
                item_id.as_ref()
            ),
        )?;
        self.client.send_unit(req).await
    }

    /// Gets a playlist's users.
    ///
    /// OpenAPI: `GET /Playlists/{playlistId}/Users` (`GetPlaylistUsers`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_playlist_users(
        &self,
        playlist_id: uuid::Uuid,
    ) -> Result<Vec<PlaylistUserPermissions>> {
        let req = self
            .client
            .request(Method::GET, &format!("Playlists/{playlist_id}/Users"))?;
        self.client.send_json(req).await
    }

    /// Gets a playlist user.
    ///
    /// OpenAPI: `GET /Playlists/{playlistId}/Users/{userId}` (`GetPlaylistUser`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_playlist_user(
        &self,
        playlist_id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Result<PlaylistUserPermissions> {
        let req = self.client.request(
            Method::GET,
            &format!("Playlists/{playlist_id}/Users/{user_id}"),
        )?;
        self.client.send_json(req).await
    }

    /// Updates a playlist user.
    ///
    /// OpenAPI: `POST /Playlists/{playlistId}/Users/{userId}` (`UpdatePlaylistUser`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn update_playlist_user(
        &self,
        playlist_id: uuid::Uuid,
        user_id: uuid::Uuid,
        request: UpdatePlaylistUser,
    ) -> Result<()> {
        let req = self
            .client
            .request(
                Method::POST,
                &format!("Playlists/{playlist_id}/Users/{user_id}"),
            )?
            .json(&request);
        self.client.send_unit(req).await
    }

    /// Removes a user from a playlist.
    ///
    /// OpenAPI: `DELETE /Playlists/{playlistId}/Users/{userId}` (`RemoveUserFromPlaylist`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn remove_user_from_playlist(
        &self,
        playlist_id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Result<()> {
        let req = self.client.request(
            Method::DELETE,
            &format!("Playlists/{playlist_id}/Users/{user_id}"),
        )?;
        self.client.send_unit(req).await
    }
}

fn push_joined<I: IntoIterator<Item = String>>(
    q: &mut Vec<(String, String)>,
    key: &str,
    values: I,
) {
    let joined = values.into_iter().collect::<Vec<_>>().join(",");
    if !joined.is_empty() {
        q.push((key.to_owned(), joined));
    }
}
