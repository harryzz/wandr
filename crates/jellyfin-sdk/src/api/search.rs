use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{BaseItemKind, MediaType, SearchHintResultStub},
};

/// Query parameters for `GET /Search/Hints`.
#[derive(Clone, Debug)]
pub struct SearchHintsQuery {
    params: Vec<(String, String)>,
    include_item_types: Vec<BaseItemKind>,
    exclude_item_types: Vec<BaseItemKind>,
    media_types: Vec<MediaType>,
}

impl SearchHintsQuery {
    /// Creates a query with the required `searchTerm`.
    pub fn new(search_term: impl Into<String>) -> Self {
        Self {
            params: vec![("searchTerm".to_owned(), search_term.into())],
            include_item_types: Vec::new(),
            exclude_item_types: Vec::new(),
            media_types: Vec::new(),
        }
    }

    /// Optional. The record index to start at.
    pub fn start_index(mut self, start_index: u32) -> Self {
        self.params
            .push(("startIndex".to_owned(), start_index.to_string()));
        self
    }

    /// Optional. The maximum number of records to return.
    pub fn limit(mut self, limit: u32) -> Self {
        self.params.push(("limit".to_owned(), limit.to_string()));
        self
    }

    /// Optional. Supply a user id to search within a user's library.
    pub fn user_id(mut self, user_id: uuid::Uuid) -> Self {
        self.params.push(("userId".to_owned(), user_id.to_string()));
        self
    }

    /// If specified, only children of the parent are returned.
    pub fn parent_id(mut self, parent_id: uuid::Uuid) -> Self {
        self.params
            .push(("parentId".to_owned(), parent_id.to_string()));
        self
    }

    /// If specified, only results with the specified item types are returned.
    pub fn include_item_type(mut self, kind: BaseItemKind) -> Self {
        self.include_item_types.push(kind);
        self
    }

    /// If specified, results with these item types are filtered out.
    pub fn exclude_item_type(mut self, kind: BaseItemKind) -> Self {
        self.exclude_item_types.push(kind);
        self
    }

    /// If specified, only results with the specified media types are returned.
    pub fn media_type(mut self, media_type: MediaType) -> Self {
        self.media_types.push(media_type);
        self
    }

    /// Optional filter for movies.
    pub fn is_movie(mut self, is_movie: bool) -> Self {
        self.params
            .push(("isMovie".to_owned(), is_movie.to_string()));
        self
    }

    /// Optional filter for series.
    pub fn is_series(mut self, is_series: bool) -> Self {
        self.params
            .push(("isSeries".to_owned(), is_series.to_string()));
        self
    }

    /// Optional filter whether to include people.
    pub fn include_people(mut self, include: bool) -> Self {
        self.params
            .push(("includePeople".to_owned(), include.to_string()));
        self
    }

    /// Optional filter whether to include media.
    pub fn include_media(mut self, include: bool) -> Self {
        self.params
            .push(("includeMedia".to_owned(), include.to_string()));
        self
    }

    /// Optional filter whether to include genres.
    pub fn include_genres(mut self, include: bool) -> Self {
        self.params
            .push(("includeGenres".to_owned(), include.to_string()));
        self
    }

    /// Optional filter whether to include studios.
    pub fn include_studios(mut self, include: bool) -> Self {
        self.params
            .push(("includeStudios".to_owned(), include.to_string()));
        self
    }

    /// Optional filter whether to include artists.
    pub fn include_artists(mut self, include: bool) -> Self {
        self.params
            .push(("includeArtists".to_owned(), include.to_string()));
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
        push_joined(
            &mut q,
            "excludeItemTypes",
            self.exclude_item_types.iter().map(|v| v.to_string()),
        );
        push_joined(
            &mut q,
            "mediaTypes",
            self.media_types.iter().map(|v| v.to_string()),
        );

        q
    }
}

/// Search-related endpoints.
#[derive(Clone, Debug)]
pub struct SearchApi {
    client: JellyfinClient,
}

impl SearchApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets the search hint result.
    ///
    /// OpenAPI: `GET /Search/Hints` (`GetSearchHints`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_search_hints(&self, query: SearchHintsQuery) -> Result<SearchHintResultStub> {
        let req = self
            .client
            .request(Method::GET, "Search/Hints")?
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
