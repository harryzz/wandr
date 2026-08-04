use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{BaseItemStub, QueryResult, SpecialViewOption},
};

/// Query parameters for `GET /UserViews`.
#[derive(Clone, Debug, Default)]
pub struct UserViewsQuery {
    params: Vec<(String, String)>,
}

impl UserViewsQuery {
    /// Creates an empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// User id.
    pub fn user_id(mut self, user_id: uuid::Uuid) -> Self {
        self.params.push(("userId".to_owned(), user_id.to_string()));
        self
    }

    /// Whether or not to include external views such as channels or live tv.
    pub fn include_external_content(mut self, include: bool) -> Self {
        self.params
            .push(("includeExternalContent".to_owned(), include.to_string()));
        self
    }

    /// Whether or not to include hidden content.
    pub fn include_hidden(mut self, include: bool) -> Self {
        self.params
            .push(("includeHidden".to_owned(), include.to_string()));
        self
    }

    /// Adds a preset view entry (wire name from `CollectionType`, e.g. `movies`, `tvshows`).
    pub fn preset_view(mut self, preset: impl Into<String>) -> Self {
        self.params.push(("presetViews".to_owned(), preset.into()));
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }
}

/// User views (top-level library categories) endpoints.
#[derive(Clone, Debug)]
pub struct UserViewsApi {
    client: JellyfinClient,
}

impl UserViewsApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets user views (top-level categories).
    ///
    /// OpenAPI: `GET /UserViews` (`GetUserViews`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_user_views(&self, query: UserViewsQuery) -> Result<QueryResult<BaseItemStub>> {
        let req = self
            .client
            .request(Method::GET, "UserViews")?
            .query(&query.params);
        self.client.send_json(req).await
    }

    /// Get user view grouping options.
    ///
    /// OpenAPI: `GET /UserViews/GroupingOptions` (`GetGroupingOptions`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_grouping_options(
        &self,
        user_id: Option<uuid::Uuid>,
    ) -> Result<Vec<SpecialViewOption>> {
        let mut req = self
            .client
            .request(Method::GET, "UserViews/GroupingOptions")?;
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        self.client.send_json(req).await
    }
}
