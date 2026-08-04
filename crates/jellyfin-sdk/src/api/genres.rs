use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{
        BaseItemKind, BaseItemStub, ImageType, ItemField, ItemSortBy, QueryResult, SortOrder,
    },
    pagination::QueryPager,
};

/// Query parameters for `GET /Genres`.
#[derive(Clone, Debug, Default)]
pub struct GenresQuery {
    params: Vec<(String, String)>,
    start_index: Option<u32>,
    limit: Option<u32>,
    fields: Vec<ItemField>,
    include_item_types: Vec<BaseItemKind>,
    exclude_item_types: Vec<BaseItemKind>,
    sort_by: Vec<ItemSortBy>,
    sort_order: Vec<SortOrder>,
    enable_image_types: Vec<ImageType>,
}

impl GenresQuery {
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

    /// The search term.
    pub fn search_term(mut self, search_term: impl Into<String>) -> Self {
        self.params
            .push(("searchTerm".to_owned(), search_term.into()));
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

    /// Whether to include image information in output.
    pub fn enable_images(mut self, enable: bool) -> Self {
        self.params
            .push(("enableImages".to_owned(), enable.to_string()));
        self
    }

    /// Whether to include total record count.
    pub fn enable_total_record_count(mut self, enable: bool) -> Self {
        self.params
            .push(("enableTotalRecordCount".to_owned(), enable.to_string()));
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

    /// Optional filter by favorite items.
    pub fn is_favorite(mut self, is_favorite: bool) -> Self {
        self.params
            .push(("isFavorite".to_owned(), is_favorite.to_string()));
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
        push_joined(
            &mut q,
            "enableImageTypes",
            self.enable_image_types.iter().map(|v| v.to_string()),
        );

        q
    }

    fn to_query_pairs(&self) -> Vec<(String, String)> {
        let mut q = self.base_params();

        if let Some(start_index) = self.start_index {
            q.push(("startIndex".to_owned(), start_index.to_string()));
        }
        if let Some(limit) = self.limit {
            q.push(("limit".to_owned(), limit.to_string()));
        }

        q
    }
}

/// Genres endpoints.
#[derive(Clone, Debug)]
pub struct GenresApi {
    client: JellyfinClient,
}

impl GenresApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets all genres from a given item, folder, or the entire library.
    ///
    /// OpenAPI: `GET /Genres` (`GetGenres`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_genres(&self, query: GenresQuery) -> Result<QueryResult<BaseItemStub>> {
        let req = self
            .client
            .request(Method::GET, "Genres")?
            .query(&query.to_query_pairs());
        self.client.send_json(req).await
    }

    /// Creates a pager over `GET /Genres`.
    pub fn pager(&self, query: GenresQuery) -> QueryPager<BaseItemStub> {
        QueryPager::new(
            self.client.clone(),
            Method::GET,
            "Genres",
            query.base_params(),
        )
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
