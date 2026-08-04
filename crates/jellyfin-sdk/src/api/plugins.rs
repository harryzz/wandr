use reqwest::Method;

use crate::{JellyfinClient, Result, models::PluginInfo};

/// Plugin management endpoints.
#[derive(Clone, Debug)]
pub struct PluginsApi {
    client: JellyfinClient,
}

impl PluginsApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets a list of currently installed plugins.
    ///
    /// OpenAPI: `GET /Plugins` (`GetPlugins`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_plugins(&self) -> Result<Vec<PluginInfo>> {
        let req = self.client.request(Method::GET, "Plugins")?;
        self.client.send_json(req).await
    }

    /// Gets plugin configuration.
    ///
    /// Plugin configuration is plugin-specific; the OpenAPI schema is `BasePluginConfiguration`
    /// (an empty object), so this SDK returns `serde_json::Value` as a forward-compatible payload.
    ///
    /// OpenAPI: `GET /Plugins/{pluginId}/Configuration` (`GetPluginConfiguration`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_plugin_configuration(
        &self,
        plugin_id: uuid::Uuid,
    ) -> Result<serde_json::Value> {
        let req = self
            .client
            .request(Method::GET, &format!("Plugins/{plugin_id}/Configuration"))?;
        self.client.send_json(req).await
    }

    /// Updates plugin configuration.
    ///
    /// OpenAPI describes this as accepting JSON body.
    ///
    /// OpenAPI: `POST /Plugins/{pluginId}/Configuration` (`UpdatePluginConfiguration`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn update_plugin_configuration(
        &self,
        plugin_id: uuid::Uuid,
        config: serde_json::Value,
    ) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, &format!("Plugins/{plugin_id}/Configuration"))?
            .json(&config);
        self.client.send_unit(req).await
    }

    /// Gets a plugin's manifest.
    ///
    /// Note: the OpenAPI spec currently declares a 204 No Content response, but servers typically
    /// return a payload; this method returns a raw response for maximum compatibility.
    ///
    /// OpenAPI: `POST /Plugins/{pluginId}/Manifest` (`GetPluginManifest`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_plugin_manifest(&self, plugin_id: uuid::Uuid) -> Result<reqwest::Response> {
        let req = self
            .client
            .request(Method::POST, &format!("Plugins/{plugin_id}/Manifest"))?;
        self.client.execute(req).await
    }

    /// Gets a plugin image (if available).
    ///
    /// OpenAPI: `GET /Plugins/{pluginId}/{version}/Image` (`GetPluginImage`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_plugin_image(
        &self,
        plugin_id: uuid::Uuid,
        version: impl AsRef<str>,
    ) -> Result<reqwest::Response> {
        let req = self.client.request(
            Method::GET,
            &format!("Plugins/{plugin_id}/{}/Image", version.as_ref()),
        )?;
        self.client.execute(req).await
    }

    /// Enables a disabled plugin.
    ///
    /// OpenAPI: `POST /Plugins/{pluginId}/{version}/Enable` (`EnablePlugin`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn enable_plugin(
        &self,
        plugin_id: uuid::Uuid,
        version: impl AsRef<str>,
    ) -> Result<()> {
        let req = self.client.request(
            Method::POST,
            &format!("Plugins/{plugin_id}/{}/Enable", version.as_ref()),
        )?;
        self.client.send_unit(req).await
    }

    /// Disables a plugin.
    ///
    /// OpenAPI: `POST /Plugins/{pluginId}/{version}/Disable` (`DisablePlugin`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn disable_plugin(
        &self,
        plugin_id: uuid::Uuid,
        version: impl AsRef<str>,
    ) -> Result<()> {
        let req = self.client.request(
            Method::POST,
            &format!("Plugins/{plugin_id}/{}/Disable", version.as_ref()),
        )?;
        self.client.send_unit(req).await
    }

    /// Uninstalls a plugin by version.
    ///
    /// OpenAPI: `DELETE /Plugins/{pluginId}/{version}` (`UninstallPluginByVersion`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn uninstall_plugin_by_version(
        &self,
        plugin_id: uuid::Uuid,
        version: impl AsRef<str>,
    ) -> Result<()> {
        let req = self.client.request(
            Method::DELETE,
            &format!("Plugins/{plugin_id}/{}", version.as_ref()),
        )?;
        self.client.send_unit(req).await
    }

    /// Uninstalls a plugin (deprecated in OpenAPI; prefer uninstalling by version).
    ///
    /// OpenAPI: `DELETE /Plugins/{pluginId}` (`UninstallPlugin`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn uninstall_plugin(&self, plugin_id: uuid::Uuid) -> Result<()> {
        let req = self
            .client
            .request(Method::DELETE, &format!("Plugins/{plugin_id}"))?;
        self.client.send_unit(req).await
    }
}
