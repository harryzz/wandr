use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{BaseItemDetailStub, BaseItemStub, ImageType, QueryResult, UserItemData},
};

/// Query parameters for `GET /Items/Latest`.
#[derive(Clone, Debug, Default)]
pub struct LatestMediaQuery {
    params: Vec<(String, String)>,
}

impl LatestMediaQuery {
    /// Creates an empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// User id.
    pub fn user_id(mut self, user_id: uuid::Uuid) -> Self {
        self.params.push(("userId".to_owned(), user_id.to_string()));
        self
    }

    /// Localize the search to a specific item or folder.
    pub fn parent_id(mut self, parent_id: uuid::Uuid) -> Self {
        self.params
            .push(("parentId".to_owned(), parent_id.to_string()));
        self
    }

    /// Return item limit.
    pub fn limit(mut self, limit: u32) -> Self {
        self.params.push(("limit".to_owned(), limit.to_string()));
        self
    }

    /// Whether or not to group items into a parent container.
    pub fn group_items(mut self, group: bool) -> Self {
        self.params
            .push(("groupItems".to_owned(), group.to_string()));
        self
    }

    /// Whether to include image information in output.
    pub fn enable_images(mut self, enable: bool) -> Self {
        self.params
            .push(("enableImages".to_owned(), enable.to_string()));
        self
    }

    /// The image types to include in the output.
    pub fn enable_image_type(mut self, image_type: ImageType) -> Self {
        self.params
            .push(("enableImageTypes".to_owned(), image_type.to_string()));
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }
}

/// Query parameters for `GET /UserItems/Resume`.
#[derive(Clone, Debug, Default)]
pub struct ResumeItemsQuery {
    params: Vec<(String, String)>,
}

impl ResumeItemsQuery {
    /// Creates an empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// User id.
    pub fn user_id(mut self, user_id: uuid::Uuid) -> Self {
        self.params.push(("userId".to_owned(), user_id.to_string()));
        self
    }

    /// The start index.
    pub fn start_index(mut self, start_index: u32) -> Self {
        self.params
            .push(("startIndex".to_owned(), start_index.to_string()));
        self
    }

    /// The item limit.
    pub fn limit(mut self, limit: u32) -> Self {
        self.params.push(("limit".to_owned(), limit.to_string()));
        self
    }

    /// The search term.
    pub fn search_term(mut self, search_term: impl Into<String>) -> Self {
        self.params
            .push(("searchTerm".to_owned(), search_term.into()));
        self
    }

    /// Localize the search to a specific item or folder.
    pub fn parent_id(mut self, parent_id: uuid::Uuid) -> Self {
        self.params
            .push(("parentId".to_owned(), parent_id.to_string()));
        self
    }

    /// Whether to include user data.
    pub fn enable_user_data(mut self, enable: bool) -> Self {
        self.params
            .push(("enableUserData".to_owned(), enable.to_string()));
        self
    }

    /// Whether to exclude currently active sessions.
    pub fn exclude_active_sessions(mut self, exclude: bool) -> Self {
        self.params
            .push(("excludeActiveSessions".to_owned(), exclude.to_string()));
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }
}

/// User library browsing endpoints.
#[derive(Clone, Debug)]
pub struct UserLibraryApi {
    client: JellyfinClient,
}

impl UserLibraryApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets intros for an item.
    ///
    /// OpenAPI: `GET /Items/{itemId}/Intros` (`GetIntros`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_intros(
        &self,
        item_id: uuid::Uuid,
        user_id: Option<uuid::Uuid>,
    ) -> Result<QueryResult<BaseItemDetailStub>> {
        let mut req = self
            .client
            .request(Method::GET, &format!("Items/{item_id}/Intros"))?;
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        self.client.send_json(req).await
    }

    /// Gets local trailers for an item.
    ///
    /// OpenAPI: `GET /Items/{itemId}/LocalTrailers` (`GetLocalTrailers`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_local_trailers(
        &self,
        item_id: uuid::Uuid,
        user_id: Option<uuid::Uuid>,
    ) -> Result<Vec<BaseItemDetailStub>> {
        let mut req = self
            .client
            .request(Method::GET, &format!("Items/{item_id}/LocalTrailers"))?;
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        self.client.send_json(req).await
    }

    /// Gets special features for an item.
    ///
    /// OpenAPI: `GET /Items/{itemId}/SpecialFeatures` (`GetSpecialFeatures`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_special_features(
        &self,
        item_id: uuid::Uuid,
        user_id: Option<uuid::Uuid>,
    ) -> Result<Vec<BaseItemDetailStub>> {
        let mut req = self
            .client
            .request(Method::GET, &format!("Items/{item_id}/SpecialFeatures"))?;
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        self.client.send_json(req).await
    }

    /// Gets the root folder from a user's library.
    ///
    /// OpenAPI: `GET /Items/Root` (`GetRootFolder`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_root_folder(&self, user_id: Option<uuid::Uuid>) -> Result<BaseItemStub> {
        let mut req = self.client.request(Method::GET, "Items/Root")?;
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        self.client.send_json(req).await
    }

    /// Marks an item as a favorite.
    ///
    /// OpenAPI: `POST /UserFavoriteItems/{itemId}` (`MarkFavoriteItem`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn mark_favorite(
        &self,
        item_id: uuid::Uuid,
        user_id: Option<uuid::Uuid>,
    ) -> Result<UserItemData> {
        let mut req = self
            .client
            .request(Method::POST, &format!("UserFavoriteItems/{item_id}"))?;
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        self.client.send_json(req).await
    }

    /// Unmarks an item as a favorite.
    ///
    /// OpenAPI: `DELETE /UserFavoriteItems/{itemId}` (`UnmarkFavoriteItem`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn unmark_favorite(
        &self,
        item_id: uuid::Uuid,
        user_id: Option<uuid::Uuid>,
    ) -> Result<UserItemData> {
        let mut req = self
            .client
            .request(Method::DELETE, &format!("UserFavoriteItems/{item_id}"))?;
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        self.client.send_json(req).await
    }

    /// Updates a user's "likes" rating for an item.
    ///
    /// OpenAPI: `POST /UserItems/{itemId}/Rating` (`UpdateUserItemRating`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn update_item_like(
        &self,
        item_id: uuid::Uuid,
        user_id: Option<uuid::Uuid>,
        likes: bool,
    ) -> Result<UserItemData> {
        let mut req = self
            .client
            .request(Method::POST, &format!("UserItems/{item_id}/Rating"))?
            .query(&[("likes", likes.to_string())]);
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        self.client.send_json(req).await
    }

    /// Deletes a user's saved personal rating for an item.
    ///
    /// OpenAPI: `DELETE /UserItems/{itemId}/Rating` (`DeleteUserItemRating`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn delete_item_rating(
        &self,
        item_id: uuid::Uuid,
        user_id: Option<uuid::Uuid>,
    ) -> Result<UserItemData> {
        let mut req = self
            .client
            .request(Method::DELETE, &format!("UserItems/{item_id}/Rating"))?;
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        self.client.send_json(req).await
    }

    /// Gets an item from a user's library.
    ///
    /// OpenAPI: `GET /Items/{itemId}` (`GetItem`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_item(
        &self,
        item_id: uuid::Uuid,
        user_id: Option<uuid::Uuid>,
    ) -> Result<BaseItemDetailStub> {
        let mut req = self
            .client
            .request(Method::GET, &format!("Items/{item_id}"))?;
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        self.client.send_json(req).await
    }

    /// Gets latest media.
    ///
    /// OpenAPI: `GET /Items/Latest` (`GetLatestMedia`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_latest_media(
        &self,
        query: LatestMediaQuery,
    ) -> Result<Vec<BaseItemDetailStub>> {
        let req = self
            .client
            .request(Method::GET, "Items/Latest")?
            .query(&query.params);
        self.client.send_json(req).await
    }

    /// Gets resume items (continue watching).
    ///
    /// OpenAPI: `GET /UserItems/Resume` (`GetResumeItems`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_resume_items(
        &self,
        query: ResumeItemsQuery,
    ) -> Result<QueryResult<BaseItemDetailStub>> {
        let req = self
            .client
            .request(Method::GET, "UserItems/Resume")?
            .query(&query.params);
        self.client.send_json(req).await
    }
}
