use reqwest::Method;

use crate::{JellyfinClient, Result};

/// Server configuration endpoints.
#[derive(Clone, Debug)]
pub struct ConfigurationApi {
    client: JellyfinClient,
}

impl ConfigurationApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets application configuration.
    ///
    /// OpenAPI: `GET /System/Configuration` (`GetConfiguration`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_configuration(&self) -> Result<serde_json::Value> {
        let req = self.client.request(Method::GET, "System/Configuration")?;
        self.client.send_json(req).await
    }

    /// Updates application configuration.
    ///
    /// OpenAPI: `POST /System/Configuration` (`UpdateConfiguration`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn update_configuration(&self, config: serde_json::Value) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, "System/Configuration")?
            .json(&config);
        self.client.send_unit(req).await
    }

    /// Updates branding configuration.
    ///
    /// OpenAPI: `POST /System/Configuration/Branding` (`UpdateBrandingConfiguration`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn update_branding_configuration(&self, branding: serde_json::Value) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, "System/Configuration/Branding")?
            .json(&branding);
        self.client.send_unit(req).await
    }

    /// Gets a default MetadataOptions object.
    ///
    /// OpenAPI: `GET /System/Configuration/MetadataOptions/Default` (`GetDefaultMetadataOptions`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_default_metadata_options(&self) -> Result<serde_json::Value> {
        let req = self
            .client
            .request(Method::GET, "System/Configuration/MetadataOptions/Default")?;
        self.client.send_json(req).await
    }

    /// Gets a named configuration.
    ///
    /// The server returns a binary payload (OpenAPI marks it as `string`/`binary`).
    ///
    /// OpenAPI: `GET /System/Configuration/{key}` (`GetNamedConfiguration`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_named_configuration(&self, key: impl AsRef<str>) -> Result<reqwest::Response> {
        let req = self.client.request(
            Method::GET,
            &format!("System/Configuration/{}", key.as_ref()),
        )?;
        self.client.execute(req).await
    }

    /// Updates named configuration.
    ///
    /// OpenAPI: `POST /System/Configuration/{key}` (`UpdateNamedConfiguration`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn update_named_configuration(
        &self,
        key: impl AsRef<str>,
        config: serde_json::Value,
    ) -> Result<()> {
        let req = self
            .client
            .request(
                Method::POST,
                &format!("System/Configuration/{}", key.as_ref()),
            )?
            .json(&config);
        self.client.send_unit(req).await
    }
}
