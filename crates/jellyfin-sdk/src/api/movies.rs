use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{ItemField, RecommendationStub},
};

/// Query parameters for `GET /Movies/Recommendations`.
#[derive(Clone, Debug, Default)]
pub struct MovieRecommendationsQuery {
    params: Vec<(String, String)>,
    fields: Vec<ItemField>,
}

impl MovieRecommendationsQuery {
    /// Creates an empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// Optional. Filter by user id, and attach user data.
    pub fn user_id(mut self, user_id: uuid::Uuid) -> Self {
        self.params.push(("userId".to_owned(), user_id.to_string()));
        self
    }

    /// Optional. Localize the search to a specific item or folder.
    pub fn parent_id(mut self, parent_id: uuid::Uuid) -> Self {
        self.params
            .push(("parentId".to_owned(), parent_id.to_string()));
        self
    }

    /// Optional. Specify additional fields of information to return in the output.
    pub fn field(mut self, field: ItemField) -> Self {
        self.fields.push(field);
        self
    }

    /// The max number of categories to return.
    pub fn category_limit(mut self, limit: u32) -> Self {
        self.params
            .push(("categoryLimit".to_owned(), limit.to_string()));
        self
    }

    /// The max number of items to return per category.
    pub fn item_limit(mut self, limit: u32) -> Self {
        self.params
            .push(("itemLimit".to_owned(), limit.to_string()));
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }

    fn to_params(&self) -> Vec<(String, String)> {
        let mut q = self.params.clone();
        push_joined(&mut q, "fields", self.fields.iter().map(|v| v.to_string()));
        q
    }
}

/// Movie-related endpoints.
#[derive(Clone, Debug)]
pub struct MoviesApi {
    client: JellyfinClient,
}

impl MoviesApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets movie recommendations.
    ///
    /// OpenAPI: `GET /Movies/Recommendations` (`GetMovieRecommendations`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_recommendations(
        &self,
        query: MovieRecommendationsQuery,
    ) -> Result<Vec<RecommendationStub>> {
        let req = self
            .client
            .request(Method::GET, "Movies/Recommendations")?
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
