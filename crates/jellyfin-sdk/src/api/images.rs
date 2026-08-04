use reqwest::Method;

use crate::{JellyfinClient, Result, models::ImageFormat};

/// Options for `GET /UserImage`.
#[derive(Clone, Debug, Default)]
pub struct UserImageRequest {
    tag: Option<String>,
    format: Option<ImageFormat>,
    extra: Vec<(String, String)>,
}

impl UserImageRequest {
    /// Creates an empty request options set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Supply the cache tag from the item object to receive strong caching headers.
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Output image format.
    pub fn format(mut self, format: ImageFormat) -> Self {
        self.format = Some(format);
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.push((key.into(), value.into()));
        self
    }

    fn to_query(&self, user_id: uuid::Uuid) -> Vec<(String, String)> {
        let mut q: Vec<(String, String)> = vec![("userId".to_owned(), user_id.to_string())];
        if let Some(tag) = &self.tag {
            q.push(("tag".to_owned(), tag.clone()));
        }
        if let Some(format) = self.format {
            q.push(("format".to_owned(), format.to_string()));
        }
        q.extend(self.extra.clone());
        q
    }
}

/// Image endpoints not directly tied to items.
#[derive(Clone, Debug)]
pub struct ImagesApi {
    client: JellyfinClient,
}

impl ImagesApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets a user profile image.
    ///
    /// OpenAPI: `GET /UserImage` (`GetUserImage`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_user_image(
        &self,
        user_id: uuid::Uuid,
        options: UserImageRequest,
    ) -> Result<reqwest::Response> {
        let req = self
            .client
            .request(Method::GET, "UserImage")?
            .query(&options.to_query(user_id));
        self.client.execute(req).await
    }

    /// Downloads a user profile image into a file.
    pub async fn download_user_image_to_file(
        &self,
        user_id: uuid::Uuid,
        options: UserImageRequest,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        let req = self
            .client
            .request(Method::GET, "UserImage")?
            .query(&options.to_query(user_id));
        self.client.download(req, file_path).await
    }

    /// Sends a HEAD request for a user profile image.
    ///
    /// OpenAPI: `HEAD /UserImage` (`HeadUserImage`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn head_user_image(
        &self,
        user_id: uuid::Uuid,
        options: UserImageRequest,
    ) -> Result<reqwest::Response> {
        let req = self
            .client
            .request(Method::HEAD, "UserImage")?
            .query(&options.to_query(user_id));
        self.client.execute(req).await
    }
}
