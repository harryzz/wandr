use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{AuthenticationInfo, QueryResult},
};

/// API key management endpoints.
#[derive(Clone, Debug)]
pub struct ApiKeyApi {
    client: JellyfinClient,
}

impl ApiKeyApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets all API keys.
    ///
    /// OpenAPI: `GET /Auth/Keys` (`GetKeys`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_keys(&self) -> Result<QueryResult<AuthenticationInfo>> {
        let req = self.client.request(Method::GET, "Auth/Keys")?;
        self.client.send_json(req).await
    }

    /// Creates a new API key.
    ///
    /// OpenAPI: `POST /Auth/Keys` (`CreateKey`).
    ///
    /// Note: the OpenAPI spec currently declares a 204 No Content response.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn create_key(&self, app: impl Into<String>) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, "Auth/Keys")?
            .query(&[("app", app.into())]);
        self.client.send_unit(req).await
    }

    /// Revokes an API key.
    ///
    /// OpenAPI: `DELETE /Auth/Keys/{key}` (`RevokeKey`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn revoke_key(&self, key: impl AsRef<str>) -> Result<()> {
        let req = self
            .client
            .request(Method::DELETE, &format!("Auth/Keys/{}", key.as_ref()))?;
        self.client.send_unit(req).await
    }
}
