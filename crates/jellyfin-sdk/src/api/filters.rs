use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{BaseItemKind, QueryFilters},
};

/// Query parameters for `GET /Items/Filters2`.
#[derive(Clone, Debug, Default)]
pub struct FiltersQuery {
    params: Vec<(String, String)>,
    include_item_types: Vec<BaseItemKind>,
}

impl FiltersQuery {
    /// Creates an empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// Optional. User id.
    pub fn user_id(mut self, user_id: uuid::Uuid) -> Self {
        self.params.push(("userId".to_owned(), user_id.to_string()));
        self
    }

    /// Optional. Specify this to localize the search to a specific item or folder.
    pub fn parent_id(mut self, parent_id: uuid::Uuid) -> Self {
        self.params
            .push(("parentId".to_owned(), parent_id.to_string()));
        self
    }

    /// Optional. Filter by item type (comma delimited).
    pub fn include_item_type(mut self, kind: BaseItemKind) -> Self {
        self.include_item_types.push(kind);
        self
    }

    /// Optional. Search recursive.
    pub fn recursive(mut self, recursive: bool) -> Self {
        self.params
            .push(("recursive".to_owned(), recursive.to_string()));
        self
    }

    /// Optional. Is item movie.
    pub fn is_movie(mut self, is_movie: bool) -> Self {
        self.params
            .push(("isMovie".to_owned(), is_movie.to_string()));
        self
    }

    /// Optional. Is item series.
    pub fn is_series(mut self, is_series: bool) -> Self {
        self.params
            .push(("isSeries".to_owned(), is_series.to_string()));
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
            "includeItemTypes",
            self.include_item_types.iter().map(|v| v.to_string()),
        );

        q
    }
}

/// Filter-related endpoints (tags/genres facets).
#[derive(Clone, Debug)]
pub struct FiltersApi {
    client: JellyfinClient,
}

impl FiltersApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets query filters (genres and tags) for building faceted browsing UIs.
    ///
    /// OpenAPI: `GET /Items/Filters2` (`GetQueryFilters`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_query_filters(&self, query: FiltersQuery) -> Result<QueryFilters> {
        let req = self
            .client
            .request(Method::GET, "Items/Filters2")?
            .query(&query.to_params());
        self.client.send_json(req).await
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
