use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{
        AddVirtualFolderBody, CollectionType, MediaPath, UpdateLibraryOptionsRequest,
        UpdateMediaPathRequest, VirtualFolderInfo,
    },
};

/// Library structure management endpoints.
#[derive(Clone, Debug)]
pub struct LibraryStructureApi {
    client: JellyfinClient,
}

impl LibraryStructureApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets all virtual folders.
    ///
    /// OpenAPI: `GET /Library/VirtualFolders` (`GetVirtualFolders`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_virtual_folders(&self) -> Result<Vec<VirtualFolderInfo>> {
        let req = self.client.request(Method::GET, "Library/VirtualFolders")?;
        self.client.send_json(req).await
    }

    /// Adds a virtual folder.
    ///
    /// OpenAPI: `POST /Library/VirtualFolders` (`AddVirtualFolder`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn add_virtual_folder(
        &self,
        name: Option<String>,
        collection_type: Option<CollectionType>,
        paths: Vec<String>,
        refresh_library: bool,
        body: Option<AddVirtualFolderBody>,
    ) -> Result<()> {
        let mut params: Vec<(String, String)> = Vec::new();
        if let Some(name) = name {
            params.push(("name".to_owned(), name));
        }
        if let Some(collection_type) = collection_type {
            params.push(("collectionType".to_owned(), collection_type.to_string()));
        }
        for path in paths {
            params.push(("paths".to_owned(), path));
        }
        params.push(("refreshLibrary".to_owned(), refresh_library.to_string()));

        let mut req = self
            .client
            .request(Method::POST, "Library/VirtualFolders")?
            .query(&params);

        if let Some(body) = body {
            req = req.json(&body);
        }

        self.client.send_unit(req).await
    }

    /// Removes a virtual folder.
    ///
    /// OpenAPI: `DELETE /Library/VirtualFolders` (`RemoveVirtualFolder`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn remove_virtual_folder(
        &self,
        name: impl Into<String>,
        refresh_library: bool,
    ) -> Result<()> {
        let req = self
            .client
            .request(Method::DELETE, "Library/VirtualFolders")?
            .query(&[
                ("name", name.into()),
                ("refreshLibrary", refresh_library.to_string()),
            ]);
        self.client.send_unit(req).await
    }

    /// Renames a virtual folder.
    ///
    /// OpenAPI: `POST /Library/VirtualFolders/Name` (`RenameVirtualFolder`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn rename_virtual_folder(
        &self,
        name: impl Into<String>,
        new_name: impl Into<String>,
        refresh_library: bool,
    ) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, "Library/VirtualFolders/Name")?
            .query(&[
                ("name", name.into()),
                ("newName", new_name.into()),
                ("refreshLibrary", refresh_library.to_string()),
            ]);
        self.client.send_unit(req).await
    }

    /// Updates library options.
    ///
    /// OpenAPI: `POST /Library/VirtualFolders/LibraryOptions` (`UpdateLibraryOptions`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn update_library_options(&self, request: UpdateLibraryOptionsRequest) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, "Library/VirtualFolders/LibraryOptions")?
            .json(&request);
        self.client.send_unit(req).await
    }

    /// Adds a media path to a library.
    ///
    /// OpenAPI: `POST /Library/VirtualFolders/Paths` (`AddMediaPath`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn add_media_path(&self, refresh_library: bool, media_path: MediaPath) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, "Library/VirtualFolders/Paths")?
            .query(&[("refreshLibrary", refresh_library.to_string())])
            .json(&media_path);
        self.client.send_unit(req).await
    }

    /// Removes a media path.
    ///
    /// OpenAPI: `DELETE /Library/VirtualFolders/Paths` (`RemoveMediaPath`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn remove_media_path(
        &self,
        library_name: impl Into<String>,
        path: impl Into<String>,
        refresh_library: bool,
    ) -> Result<()> {
        let req = self
            .client
            .request(Method::DELETE, "Library/VirtualFolders/Paths")?
            .query(&[
                ("name", library_name.into()),
                ("path", path.into()),
                ("refreshLibrary", refresh_library.to_string()),
            ]);
        self.client.send_unit(req).await
    }

    /// Updates a media path.
    ///
    /// OpenAPI: `POST /Library/VirtualFolders/Paths/Update` (`UpdateMediaPath`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn update_media_path(&self, request: UpdateMediaPathRequest) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, "Library/VirtualFolders/Paths/Update")?
            .json(&request);
        self.client.send_unit(req).await
    }
}
