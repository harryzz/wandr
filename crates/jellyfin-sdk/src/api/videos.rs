use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{PlaybackInfoResponseStub, VideoStreamQuery},
};

/// Videos-related endpoints.
#[derive(Clone, Debug)]
pub struct VideosApi {
    client: JellyfinClient,
}

impl VideosApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets a video stream.
    ///
    /// OpenAPI: `GET /Videos/{itemId}/stream` (`GetVideoStream`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_video_stream(
        &self,
        item_id: uuid::Uuid,
        query: VideoStreamQuery,
    ) -> Result<reqwest::Response> {
        let mut params = query.to_params();
        if !query.has_device_id() {
            params.push(("deviceId".to_owned(), self.client.device_id().to_owned()));
        }

        let req = self
            .client
            .request(Method::GET, &format!("Videos/{item_id}/stream"))?
            .query(&params);
        self.client.execute(req).await
    }

    /// Issues a `HEAD` request for a video stream.
    ///
    /// OpenAPI: `HEAD /Videos/{itemId}/stream` (`HeadVideoStream`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn head_video_stream(
        &self,
        item_id: uuid::Uuid,
        query: VideoStreamQuery,
    ) -> Result<reqwest::Response> {
        let mut params = query.to_params();
        if !query.has_device_id() {
            params.push(("deviceId".to_owned(), self.client.device_id().to_owned()));
        }

        let req = self
            .client
            .request(Method::HEAD, &format!("Videos/{item_id}/stream"))?
            .query(&params);
        self.client.execute(req).await
    }

    /// Gets a video stream using a container extension in the path.
    ///
    /// OpenAPI: `GET /Videos/{itemId}/stream.{container}` (`GetVideoStreamByContainer`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_video_stream_by_container(
        &self,
        item_id: uuid::Uuid,
        container: &str,
        query: VideoStreamQuery,
    ) -> Result<reqwest::Response> {
        let mut params = query.to_params_without_container();
        if !query.has_device_id() {
            params.push(("deviceId".to_owned(), self.client.device_id().to_owned()));
        }

        let req = self
            .client
            .request(Method::GET, &format!("Videos/{item_id}/stream.{container}"))?
            .query(&params);
        self.client.execute(req).await
    }

    /// Issues a `HEAD` request for `GET /Videos/{itemId}/stream.{container}`.
    ///
    /// OpenAPI: `HEAD /Videos/{itemId}/stream.{container}` (`HeadVideoStreamByContainer`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn head_video_stream_by_container(
        &self,
        item_id: uuid::Uuid,
        container: &str,
        query: VideoStreamQuery,
    ) -> Result<reqwest::Response> {
        let mut params = query.to_params_without_container();
        if !query.has_device_id() {
            params.push(("deviceId".to_owned(), self.client.device_id().to_owned()));
        }

        let req = self
            .client
            .request(
                Method::HEAD,
                &format!("Videos/{item_id}/stream.{container}"),
            )?
            .query(&params);
        self.client.execute(req).await
    }

    /// Downloads a video stream into a file.
    pub async fn download_video_stream_to_file(
        &self,
        item_id: uuid::Uuid,
        query: VideoStreamQuery,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        let mut params = query.to_params();
        if !query.has_device_id() {
            params.push(("deviceId".to_owned(), self.client.device_id().to_owned()));
        }

        let req = self
            .client
            .request(Method::GET, &format!("Videos/{item_id}/stream"))?
            .query(&params);
        self.client.download(req, file_path).await
    }

    /// Downloads a video stream using a container extension in the path.
    pub async fn download_video_stream_by_container_to_file(
        &self,
        item_id: uuid::Uuid,
        container: &str,
        query: VideoStreamQuery,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        let mut params = query.to_params_without_container();
        if !query.has_device_id() {
            params.push(("deviceId".to_owned(), self.client.device_id().to_owned()));
        }

        let req = self
            .client
            .request(Method::GET, &format!("Videos/{item_id}/stream.{container}"))?
            .query(&params);
        self.client.download(req, file_path).await
    }

    /// Streams a video using playback info as guidance.
    ///
    /// This method prefers server-provided transcoding URLs when present, otherwise it
    /// falls back to direct/static streaming of the selected media source.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn stream_from_playback_info(
        &self,
        item_id: uuid::Uuid,
        playback: &PlaybackInfoResponseStub,
    ) -> Result<reqwest::Response> {
        if let Some(first) = playback.media_sources.first()
            && let Some(url) = first.transcoding_url.as_deref()
        {
            let req = self.client.request(Method::GET, url)?;
            return self.client.execute(req).await;
        }

        let mut q = VideoStreamQuery::new().static_stream(true);
        if let Some(play_session_id) = playback.play_session_id.clone() {
            q = q.play_session_id(play_session_id);
        }
        if let Some(media_source_id) = playback.media_sources.first().and_then(|s| s.id.clone()) {
            q = q.media_source_id(media_source_id);
        }

        self.get_video_stream(item_id, q).await
    }
}
