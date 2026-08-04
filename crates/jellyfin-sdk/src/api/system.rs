use reqwest::Method;

use crate::{JellyfinClient, Result, models::PublicSystemInfo};

/// System-related endpoints.
#[derive(Clone, Debug)]
pub struct SystemApi {
    client: JellyfinClient,
}

impl SystemApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets public information about the server.
    ///
    /// OpenAPI: `GET /System/Info/Public` (`GetPublicSystemInfo`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_public_info(&self) -> Result<PublicSystemInfo> {
        let req = self.client.request(Method::GET, "System/Info/Public")?;
        self.client.send_json(req).await
    }

    /// Pings the server.
    ///
    /// OpenAPI: `GET /System/Ping` (`GetPingSystem`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn ping(&self) -> Result<String> {
        let req = self.client.request(Method::GET, "System/Ping")?;
        self.client.send_json(req).await
    }
}
