use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{AuthenticateUserByName, AuthenticationResult, UserStub},
};

/// User/authentication related endpoints.
#[derive(Clone, Debug)]
pub struct UserApi {
    client: JellyfinClient,
}

impl UserApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Authenticates a user by name.
    ///
    /// OpenAPI: `POST /Users/AuthenticateByName` (`AuthenticateUserByName`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn authenticate_by_name(
        &self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<AuthenticationResult> {
        let body = AuthenticateUserByName {
            username: Some(username.into()),
            pw: Some(password.into()),
        };

        let req = self
            .client
            .request(Method::POST, "Users/AuthenticateByName")?
            .json(&body);

        self.client.send_json(req).await
    }

    /// Authenticates a user by name and updates the client's token if one is returned.
    ///
    /// This is the most ergonomic login flow for typical SDK users:
    /// - call once to obtain an access token
    /// - subsequent requests automatically include the token
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn authenticate_by_name_and_set_token(
        &self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<AuthenticationResult> {
        let result = self.authenticate_by_name(username, password).await?;
        if let Some(token) = result.access_token.clone() {
            self.client.set_token(token);
        }
        Ok(result)
    }

    /// Gets the current user based on the auth token.
    ///
    /// OpenAPI: `GET /Users/Me` (`GetCurrentUser`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn me(&self) -> Result<UserStub> {
        let req = self.client.request(Method::GET, "Users/Me")?;
        self.client.send_json(req).await
    }

    /// Gets the current user as raw JSON.
    ///
    /// This is useful when you need fields that are not modeled yet.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn me_raw(&self) -> Result<serde_json::Value> {
        let req = self.client.request(Method::GET, "Users/Me")?;
        self.client.send_json(req).await
    }
}
