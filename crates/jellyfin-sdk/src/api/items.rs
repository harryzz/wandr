use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{
        BaseItemDetailStub, BaseItemKind, BaseItemStub, ImageFormat, ImageInfo, ImageType,
        ItemField, ItemSortBy, MetadataRefreshMode, PlaybackInfoRequest, PlaybackInfoResponseStub,
        QueryResult, SortOrder, UpdateUserItemData, UserItemData,
    },
    pagination::QueryPager,
};

/// Query parameters for `GET /Items`.
#[derive(Clone, Debug, Default)]
pub struct ItemsQuery {
    params: Vec<(String, String)>,
    start_index: Option<u32>,
    limit: Option<u32>,
    fields: Vec<ItemField>,
    include_item_types: Vec<BaseItemKind>,
    exclude_item_types: Vec<BaseItemKind>,
    sort_by: Vec<ItemSortBy>,
    sort_order: Vec<SortOrder>,
    genres: Vec<String>,
    genre_ids: Vec<uuid::Uuid>,
    tags: Vec<String>,
}

/// Query parameters for `GET /Items/{itemId}/Similar`.
#[derive(Clone, Debug, Default)]
pub struct SimilarItemsQuery {
    params: Vec<(String, String)>,
    exclude_artist_ids: Vec<uuid::Uuid>,
    fields: Vec<ItemField>,
}

/// Query parameters for `POST /Items/{itemId}/Refresh`.
#[derive(Clone, Debug, Default)]
pub struct RefreshItemQuery {
    params: Vec<(String, String)>,
    metadata_refresh_mode: Option<MetadataRefreshMode>,
    image_refresh_mode: Option<MetadataRefreshMode>,
    replace_all_metadata: Option<bool>,
    replace_all_images: Option<bool>,
    regenerate_trickplay: Option<bool>,
}

impl RefreshItemQuery {
    /// Creates an empty query (server defaults apply).
    pub fn new() -> Self {
        Self::default()
    }

    /// Specifies the metadata refresh mode.
    pub fn metadata_refresh_mode(mut self, mode: MetadataRefreshMode) -> Self {
        self.metadata_refresh_mode = Some(mode);
        self
    }

    /// Specifies the image refresh mode.
    pub fn image_refresh_mode(mut self, mode: MetadataRefreshMode) -> Self {
        self.image_refresh_mode = Some(mode);
        self
    }

    /// Determines if metadata should be replaced (only applicable if mode is `FullRefresh`).
    pub fn replace_all_metadata(mut self, replace: bool) -> Self {
        self.replace_all_metadata = Some(replace);
        self
    }

    /// Determines if images should be replaced (only applicable if mode is `FullRefresh`).
    pub fn replace_all_images(mut self, replace: bool) -> Self {
        self.replace_all_images = Some(replace);
        self
    }

    /// Determines if trickplay images should be regenerated (only applicable if mode is `FullRefresh`).
    pub fn regenerate_trickplay(mut self, regenerate: bool) -> Self {
        self.regenerate_trickplay = Some(regenerate);
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }

    fn to_params(&self) -> Vec<(String, String)> {
        let mut q = self.params.clone();

        if let Some(mode) = self.metadata_refresh_mode {
            q.push(("metadataRefreshMode".to_owned(), mode.to_string()));
        }
        if let Some(mode) = self.image_refresh_mode {
            q.push(("imageRefreshMode".to_owned(), mode.to_string()));
        }
        if let Some(v) = self.replace_all_metadata {
            q.push(("replaceAllMetadata".to_owned(), v.to_string()));
        }
        if let Some(v) = self.replace_all_images {
            q.push(("replaceAllImages".to_owned(), v.to_string()));
        }
        if let Some(v) = self.regenerate_trickplay {
            q.push(("regenerateTrickplay".to_owned(), v.to_string()));
        }

        q
    }
}

impl SimilarItemsQuery {
    /// Creates an empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// Optional. Exclude artist ids.
    pub fn exclude_artist_id(mut self, artist_id: uuid::Uuid) -> Self {
        self.exclude_artist_ids.push(artist_id);
        self
    }

    /// Optional. Filter by user id, and attach user data.
    pub fn user_id(mut self, user_id: uuid::Uuid) -> Self {
        self.params.push(("userId".to_owned(), user_id.to_string()));
        self
    }

    /// Optional. The maximum number of records to return.
    pub fn limit(mut self, limit: u32) -> Self {
        self.params.push(("limit".to_owned(), limit.to_string()));
        self
    }

    /// Optional. Specify additional fields of information to return in the output.
    pub fn field(mut self, field: ItemField) -> Self {
        self.fields.push(field);
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }

    fn to_params(&self) -> Vec<(String, String)> {
        let mut q = self.params.clone();

        push_joined(
            &mut q,
            "excludeArtistIds",
            self.exclude_artist_ids.iter().map(|v| v.to_string()),
        );
        push_joined(&mut q, "fields", self.fields.iter().map(|v| v.to_string()));

        q
    }
}

impl ItemsQuery {
    /// Creates an empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// The user id; required when not using an API key.
    pub fn user_id(mut self, user_id: uuid::Uuid) -> Self {
        self.params.push(("userId".to_owned(), user_id.to_string()));
        self
    }

    /// Filters by parent id.
    pub fn parent_id(mut self, parent_id: uuid::Uuid) -> Self {
        self.params
            .push(("parentId".to_owned(), parent_id.to_string()));
        self
    }

    /// Whether to include items recursively.
    pub fn recursive(mut self, recursive: bool) -> Self {
        self.params
            .push(("recursive".to_owned(), recursive.to_string()));
        self
    }

    /// Free-text search term.
    pub fn search_term(mut self, search_term: impl Into<String>) -> Self {
        self.params
            .push(("searchTerm".to_owned(), search_term.into()));
        self
    }

    /// Specifies additional fields of information to return.
    pub fn field(mut self, field: ItemField) -> Self {
        self.fields.push(field);
        self
    }

    /// Filters in based on item type.
    pub fn include_item_type(mut self, kind: BaseItemKind) -> Self {
        self.include_item_types.push(kind);
        self
    }

    /// Filters out based on item type.
    pub fn exclude_item_type(mut self, kind: BaseItemKind) -> Self {
        self.exclude_item_types.push(kind);
        self
    }

    /// Sorts by the given field.
    pub fn sort_by(mut self, sort: ItemSortBy) -> Self {
        self.sort_by.push(sort);
        self
    }

    /// Sort order.
    pub fn sort_order(mut self, order: SortOrder) -> Self {
        self.sort_order.push(order);
        self
    }

    /// Filters by genre name.
    pub fn genre(mut self, genre: impl Into<String>) -> Self {
        self.genres.push(genre.into());
        self
    }

    /// Filters by genre id.
    pub fn genre_id(mut self, genre_id: uuid::Uuid) -> Self {
        self.genre_ids.push(genre_id);
        self
    }

    /// Filters by tag.
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Sets `startIndex` for a single `get_items` call.
    pub fn start_index(mut self, start_index: u32) -> Self {
        self.start_index = Some(start_index);
        self
    }

    /// Sets `limit` for a single `get_items` call.
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }

    fn base_params(&self) -> Vec<(String, String)> {
        let mut q = self.params.clone();

        push_joined(&mut q, "fields", self.fields.iter().map(|v| v.to_string()));
        push_joined(
            &mut q,
            "includeItemTypes",
            self.include_item_types.iter().map(|v| v.to_string()),
        );
        push_joined(
            &mut q,
            "excludeItemTypes",
            self.exclude_item_types.iter().map(|v| v.to_string()),
        );
        push_joined(&mut q, "sortBy", self.sort_by.iter().map(|v| v.to_string()));
        push_joined(
            &mut q,
            "sortOrder",
            self.sort_order.iter().map(|v| v.to_string()),
        );
        push_joined(&mut q, "genres", self.genres.iter().cloned());
        push_joined(
            &mut q,
            "genreIds",
            self.genre_ids.iter().map(|v| v.to_string()),
        );
        push_joined(&mut q, "tags", self.tags.iter().cloned());

        q
    }
}

/// Items-related endpoints.
#[derive(Clone, Debug)]
pub struct ItemsApi {
    client: JellyfinClient,
}

/// Options for `GET /Items/{itemId}/Images/{imageType}`.
#[derive(Clone, Debug, Default)]
pub struct ItemImageRequest {
    max_width: Option<u32>,
    max_height: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
    quality: Option<u32>,
    tag: Option<String>,
    format: Option<ImageFormat>,
    percent_played: Option<f64>,
    unplayed_count: Option<u32>,
    blur: Option<u32>,
    background_color: Option<String>,
    foreground_layer: Option<String>,
    image_index: Option<u32>,
    extra: Vec<(String, String)>,
}

impl ItemImageRequest {
    /// Creates an empty request options set.
    pub fn new() -> Self {
        Self::default()
    }

    /// The maximum image width to return.
    pub fn max_width(mut self, value: u32) -> Self {
        self.max_width = Some(value);
        self
    }

    /// The maximum image height to return.
    pub fn max_height(mut self, value: u32) -> Self {
        self.max_height = Some(value);
        self
    }

    /// The fixed image width to return.
    pub fn width(mut self, value: u32) -> Self {
        self.width = Some(value);
        self
    }

    /// The fixed image height to return.
    pub fn height(mut self, value: u32) -> Self {
        self.height = Some(value);
        self
    }

    /// Quality setting, from 0-100.
    pub fn quality(mut self, value: u32) -> Self {
        self.quality = Some(value.min(100));
        self
    }

    /// Supply the cache tag to receive strong caching headers.
    pub fn tag(mut self, value: impl Into<String>) -> Self {
        self.tag = Some(value.into());
        self
    }

    /// Output image format.
    pub fn format(mut self, value: ImageFormat) -> Self {
        self.format = Some(value);
        self
    }

    /// Percent to render for the percent played overlay.
    pub fn percent_played(mut self, value: f64) -> Self {
        self.percent_played = Some(value);
        self
    }

    /// Unplayed count overlay to render.
    pub fn unplayed_count(mut self, value: u32) -> Self {
        self.unplayed_count = Some(value);
        self
    }

    /// Blur image.
    pub fn blur(mut self, value: u32) -> Self {
        self.blur = Some(value);
        self
    }

    /// Apply a background color for transparent images.
    pub fn background_color(mut self, value: impl Into<String>) -> Self {
        self.background_color = Some(value.into());
        self
    }

    /// Apply a foreground layer on top of the image.
    pub fn foreground_layer(mut self, value: impl Into<String>) -> Self {
        self.foreground_layer = Some(value.into());
        self
    }

    /// Image index.
    pub fn image_index(mut self, value: u32) -> Self {
        self.image_index = Some(value);
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.push((key.into(), value.into()));
        self
    }

    fn to_query(&self) -> Vec<(String, String)> {
        let mut q = Vec::new();

        if let Some(v) = self.max_width {
            q.push(("maxWidth".to_owned(), v.to_string()));
        }
        if let Some(v) = self.max_height {
            q.push(("maxHeight".to_owned(), v.to_string()));
        }
        if let Some(v) = self.width {
            q.push(("width".to_owned(), v.to_string()));
        }
        if let Some(v) = self.height {
            q.push(("height".to_owned(), v.to_string()));
        }
        if let Some(v) = self.quality {
            q.push(("quality".to_owned(), v.to_string()));
        }
        if let Some(v) = &self.tag {
            q.push(("tag".to_owned(), v.clone()));
        }
        if let Some(v) = self.format {
            q.push(("format".to_owned(), v.to_string()));
        }
        if let Some(v) = self.percent_played {
            q.push(("percentPlayed".to_owned(), v.to_string()));
        }
        if let Some(v) = self.unplayed_count {
            q.push(("unplayedCount".to_owned(), v.to_string()));
        }
        if let Some(v) = self.blur {
            q.push(("blur".to_owned(), v.to_string()));
        }
        if let Some(v) = &self.background_color {
            q.push(("backgroundColor".to_owned(), v.clone()));
        }
        if let Some(v) = &self.foreground_layer {
            q.push(("foregroundLayer".to_owned(), v.clone()));
        }
        if let Some(v) = self.image_index {
            q.push(("imageIndex".to_owned(), v.to_string()));
        }

        q.extend(self.extra.clone());
        q
    }
}

impl ItemsApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets an item by id.
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

    /// Gets similar items.
    ///
    /// OpenAPI: `GET /Items/{itemId}/Similar` (`GetSimilarItems`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_similar_items(
        &self,
        item_id: uuid::Uuid,
        query: SimilarItemsQuery,
    ) -> Result<QueryResult<BaseItemDetailStub>> {
        let req = self
            .client
            .request(Method::GET, &format!("Items/{item_id}/Similar"))?
            .query(&query.to_params());
        self.client.send_json(req).await
    }

    /// Refreshes metadata for an item.
    ///
    /// OpenAPI: `POST /Items/{itemId}/Refresh` (`RefreshItem`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn refresh_item(&self, item_id: uuid::Uuid, query: RefreshItemQuery) -> Result<()> {
        let mut req = self
            .client
            .request(Method::POST, &format!("Items/{item_id}/Refresh"))?;
        let params = query.to_params();
        if !params.is_empty() {
            req = req.query(&params);
        }
        self.client.send_unit(req).await
    }

    /// Gets playback info for an item.
    ///
    /// OpenAPI: `GET /Items/{itemId}/PlaybackInfo` (`GetPlaybackInfo`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_playback_info(
        &self,
        item_id: uuid::Uuid,
        user_id: Option<uuid::Uuid>,
    ) -> Result<PlaybackInfoResponseStub> {
        let mut req = self
            .client
            .request(Method::GET, &format!("Items/{item_id}/PlaybackInfo"))?;
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        self.client.send_json(req).await
    }

    /// Gets playback info for an item, using a request body.
    ///
    /// OpenAPI: `POST /Items/{itemId}/PlaybackInfo` (`GetPostedPlaybackInfo`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn post_playback_info(
        &self,
        item_id: uuid::Uuid,
        request: PlaybackInfoRequest,
    ) -> Result<PlaybackInfoResponseStub> {
        let req = self
            .client
            .request(Method::POST, &format!("Items/{item_id}/PlaybackInfo"))?
            .json(&request);
        self.client.send_json(req).await
    }

    /// Gets per-user data for an item.
    ///
    /// OpenAPI: `GET /UserItems/{itemId}/UserData` (`GetItemUserData`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_user_data(
        &self,
        item_id: uuid::Uuid,
        user_id: Option<uuid::Uuid>,
    ) -> Result<UserItemData> {
        let mut req = self
            .client
            .request(Method::GET, &format!("UserItems/{item_id}/UserData"))?;
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        self.client.send_json(req).await
    }

    /// Updates per-user data for an item.
    ///
    /// OpenAPI: `POST /UserItems/{itemId}/UserData` (`UpdateItemUserData`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn update_user_data(
        &self,
        item_id: uuid::Uuid,
        user_id: Option<uuid::Uuid>,
        update: UpdateUserItemData,
    ) -> Result<UserItemData> {
        let mut req = self
            .client
            .request(Method::POST, &format!("UserItems/{item_id}/UserData"))?
            .json(&update);
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        self.client.send_json(req).await
    }

    /// Gets all parents of an item.
    ///
    /// OpenAPI: `GET /Items/{itemId}/Ancestors` (`GetAncestors`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_ancestors(
        &self,
        item_id: uuid::Uuid,
        user_id: Option<uuid::Uuid>,
    ) -> Result<Vec<BaseItemStub>> {
        let mut req = self
            .client
            .request(Method::GET, &format!("Items/{item_id}/Ancestors"))?;
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        self.client.send_json(req).await
    }

    /// Downloads item media.
    ///
    /// OpenAPI: `GET /Items/{itemId}/Download` (`GetDownload`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn download_item(&self, item_id: uuid::Uuid) -> Result<reqwest::Response> {
        let req = self
            .client
            .request(Method::GET, &format!("Items/{item_id}/Download"))?;
        self.client.execute(req).await
    }

    /// Downloads item media into a file.
    pub async fn download_item_to_file(
        &self,
        item_id: uuid::Uuid,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        let req = self
            .client
            .request(Method::GET, &format!("Items/{item_id}/Download"))?;
        self.client.download(req, file_path).await
    }

    /// Gets the original file of an item.
    ///
    /// OpenAPI: `GET /Items/{itemId}/File` (`GetFile`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_file(&self, item_id: uuid::Uuid) -> Result<reqwest::Response> {
        let req = self
            .client
            .request(Method::GET, &format!("Items/{item_id}/File"))?;
        self.client.execute(req).await
    }

    /// Downloads the original file of an item into a file.
    pub async fn download_file_to_path(
        &self,
        item_id: uuid::Uuid,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        let req = self
            .client
            .request(Method::GET, &format!("Items/{item_id}/File"))?;
        self.client.download(req, file_path).await
    }

    /// Gets items based on a query.
    ///
    /// OpenAPI: `GET /Items` (`GetItems`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_items(&self, query: ItemsQuery) -> Result<QueryResult<BaseItemStub>> {
        let mut params = query.base_params();
        if let Some(start_index) = query.start_index {
            params.push(("startIndex".to_owned(), start_index.to_string()));
        }
        if let Some(limit) = query.limit {
            params.push(("limit".to_owned(), limit.to_string()));
        }

        let req = self.client.request(Method::GET, "Items")?.query(&params);

        self.client.send_json(req).await
    }

    /// Creates a pager over `GET /Items`.
    ///
    /// The pager handles `startIndex`/`limit` and stops when the server returns an empty page
    /// (or when `total_record_count` is reached).
    pub fn pager(&self, query: ItemsQuery) -> QueryPager<BaseItemStub> {
        QueryPager::new(
            self.client.clone(),
            Method::GET,
            "Items",
            query.base_params(),
        )
    }

    /// Gets an item's image.
    ///
    /// OpenAPI: `GET /Items/{itemId}/Images/{imageType}` (`GetItemImage`).
    pub async fn get_image(
        &self,
        item_id: uuid::Uuid,
        image_type: ImageType,
        options: ItemImageRequest,
    ) -> Result<reqwest::Response> {
        let path = format!("Items/{item_id}/Images/{}", image_type.as_str());
        let req = self
            .client
            .request(Method::GET, &path)?
            .query(&options.to_query());
        self.client.execute(req).await
    }

    /// Sends a HEAD request for an item's image.
    ///
    /// OpenAPI: `HEAD /Items/{itemId}/Images/{imageType}` (`HeadItemImage`).
    pub async fn head_image(
        &self,
        item_id: uuid::Uuid,
        image_type: ImageType,
        options: ItemImageRequest,
    ) -> Result<reqwest::Response> {
        let path = format!("Items/{item_id}/Images/{}", image_type.as_str());
        let req = self
            .client
            .request(Method::HEAD, &path)?
            .query(&options.to_query());
        self.client.execute(req).await
    }

    /// Gets an item's image by index.
    ///
    /// OpenAPI: `GET /Items/{itemId}/Images/{imageType}/{imageIndex}` (`GetItemImageByIndex`).
    pub async fn get_image_by_index(
        &self,
        item_id: uuid::Uuid,
        image_type: ImageType,
        image_index: i32,
        options: ItemImageRequest,
    ) -> Result<reqwest::Response> {
        let path = format!(
            "Items/{item_id}/Images/{}/{image_index}",
            image_type.as_str()
        );
        let req = self
            .client
            .request(Method::GET, &path)?
            .query(&options.to_query());
        self.client.execute(req).await
    }

    /// Sends a HEAD request for an item's image by index.
    ///
    /// OpenAPI: `HEAD /Items/{itemId}/Images/{imageType}/{imageIndex}` (`HeadItemImageByIndex`).
    pub async fn head_image_by_index(
        &self,
        item_id: uuid::Uuid,
        image_type: ImageType,
        image_index: i32,
        options: ItemImageRequest,
    ) -> Result<reqwest::Response> {
        let path = format!(
            "Items/{item_id}/Images/{}/{image_index}",
            image_type.as_str()
        );
        let req = self
            .client
            .request(Method::HEAD, &path)?
            .query(&options.to_query());
        self.client.execute(req).await
    }

    /// Gets image metadata for an item.
    ///
    /// OpenAPI: `GET /Items/{itemId}/Images` (`GetItemImageInfos`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_image_infos(&self, item_id: uuid::Uuid) -> Result<Vec<ImageInfo>> {
        let req = self
            .client
            .request(Method::GET, &format!("Items/{item_id}/Images"))?;
        self.client.send_json(req).await
    }

    /// Downloads an item's image into a file.
    pub async fn download_image_to_file(
        &self,
        item_id: uuid::Uuid,
        image_type: ImageType,
        options: ItemImageRequest,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        let path = format!("Items/{item_id}/Images/{}", image_type.as_str());
        let req = self
            .client
            .request(Method::GET, &path)?
            .query(&options.to_query());
        self.client.download(req, file_path).await
    }

    /// Downloads an item's image by index into a file.
    pub async fn download_image_by_index_to_file(
        &self,
        item_id: uuid::Uuid,
        image_type: ImageType,
        image_index: i32,
        options: ItemImageRequest,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        let path = format!(
            "Items/{item_id}/Images/{}/{image_index}",
            image_type.as_str()
        );
        let req = self
            .client
            .request(Method::GET, &path)?
            .query(&options.to_query());
        self.client.download(req, file_path).await
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
