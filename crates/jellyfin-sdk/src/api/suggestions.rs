use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{BaseItemKind, BaseItemStub, MediaType, QueryResult},
    pagination::QueryPager,
};

/// Query parameters for `GET /Items/Suggestions`.
#[derive(Clone, Debug, Default)]
pub struct SuggestionsQuery {
    params: Vec<(String, String)>,
    start_index: Option<u32>,
    limit: Option<u32>,
    media_types: Vec<MediaType>,
    item_types: Vec<BaseItemKind>,
}

impl SuggestionsQuery {
    /// Creates an empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// The user id.
    pub fn user_id(mut self, user_id: uuid::Uuid) -> Self {
        self.params.push(("userId".to_owned(), user_id.to_string()));
        self
    }

    /// Adds a media type filter.
    pub fn media_type(mut self, media_type: MediaType) -> Self {
        self.media_types.push(media_type);
        self
    }

    /// Adds an item type filter.
    pub fn item_type(mut self, kind: BaseItemKind) -> Self {
        self.item_types.push(kind);
        self
    }

    /// Sets `startIndex` for a single call.
    pub fn start_index(mut self, start_index: u32) -> Self {
        self.start_index = Some(start_index);
        self
    }

    /// Sets `limit` for a single call.
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Whether to enable the total record count.
    pub fn enable_total_record_count(mut self, enable: bool) -> Self {
        self.params
            .push(("enableTotalRecordCount".to_owned(), enable.to_string()));
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }

    fn base_params(&self) -> Vec<(String, String)> {
        let mut q = self.params.clone();
        push_joined(
            &mut q,
            "mediaType",
            self.media_types.iter().map(|v| v.to_string()),
        );
        push_joined(
            &mut q,
            "type",
            self.item_types.iter().map(|v| v.to_string()),
        );
        q
    }
}

/// Suggestions endpoints.
#[derive(Clone, Debug)]
pub struct SuggestionsApi {
    client: JellyfinClient,
}

impl SuggestionsApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets suggestions.
    ///
    /// OpenAPI: `GET /Items/Suggestions` (`GetSuggestions`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_suggestions(
        &self,
        query: SuggestionsQuery,
    ) -> Result<QueryResult<BaseItemStub>> {
        let mut params = query.base_params();
        if let Some(start_index) = query.start_index {
            params.push(("startIndex".to_owned(), start_index.to_string()));
        }
        if let Some(limit) = query.limit {
            params.push(("limit".to_owned(), limit.to_string()));
        }

        let req = self
            .client
            .request(Method::GET, "Items/Suggestions")?
            .query(&params);
        self.client.send_json(req).await
    }

    /// Creates a pager over `GET /Items/Suggestions`.
    pub fn pager(&self, query: SuggestionsQuery) -> QueryPager<BaseItemStub> {
        QueryPager::new(
            self.client.clone(),
            Method::GET,
            "Items/Suggestions",
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
