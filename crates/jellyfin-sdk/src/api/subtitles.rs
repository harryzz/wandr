use reqwest::Method;

use base64::Engine;

use crate::{
    JellyfinClient, Result,
    models::{
        FontFile, PlaybackInfoResponseStub, RemoteSubtitleInfo, SubtitleFormat,
        SubtitleStreamQuery, UploadSubtitleRequest,
    },
};

/// Subtitle-related endpoints.
#[derive(Clone, Debug)]
pub struct SubtitlesApi {
    client: JellyfinClient,
}

impl SubtitlesApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets a list of available fallback font files.
    ///
    /// OpenAPI: `GET /FallbackFont/Fonts` (`GetFallbackFontList`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_fallback_font_list(&self) -> Result<Vec<FontFile>> {
        let req = self.client.request(Method::GET, "FallbackFont/Fonts")?;
        self.client.send_json(req).await
    }

    /// Gets a fallback font file.
    ///
    /// OpenAPI: `GET /FallbackFont/Fonts/{name}` (`GetFallbackFont`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_fallback_font(&self, name: impl AsRef<str>) -> Result<reqwest::Response> {
        let req = self.client.request(
            Method::GET,
            &format!("FallbackFont/Fonts/{}", name.as_ref()),
        )?;
        self.client.execute(req).await
    }

    /// Downloads a fallback font file into a file.
    pub async fn download_fallback_font_to_file(
        &self,
        name: impl AsRef<str>,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        let req = self.client.request(
            Method::GET,
            &format!("FallbackFont/Fonts/{}", name.as_ref()),
        )?;
        self.client.download(req, file_path).await
    }

    /// Searches remote subtitles for an item.
    ///
    /// OpenAPI: `GET /Items/{itemId}/RemoteSearch/Subtitles/{language}` (`SearchRemoteSubtitles`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn search_remote_subtitles(
        &self,
        item_id: uuid::Uuid,
        language: impl AsRef<str>,
        is_perfect_match: Option<bool>,
    ) -> Result<Vec<RemoteSubtitleInfo>> {
        let mut req = self.client.request(
            Method::GET,
            &format!(
                "Items/{item_id}/RemoteSearch/Subtitles/{}",
                language.as_ref()
            ),
        )?;
        if let Some(is_perfect_match) = is_perfect_match {
            req = req.query(&[("isPerfectMatch", is_perfect_match.to_string())]);
        }
        self.client.send_json(req).await
    }

    /// Downloads a remote subtitle and attaches it to the item.
    ///
    /// OpenAPI: `POST /Items/{itemId}/RemoteSearch/Subtitles/{subtitleId}` (`DownloadRemoteSubtitles`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn download_remote_subtitle(
        &self,
        item_id: uuid::Uuid,
        subtitle_id: impl AsRef<str>,
    ) -> Result<()> {
        let req = self.client.request(
            Method::POST,
            &format!(
                "Items/{item_id}/RemoteSearch/Subtitles/{}",
                subtitle_id.as_ref()
            ),
        )?;
        self.client.send_unit(req).await
    }

    /// Gets the remote subtitle file.
    ///
    /// OpenAPI: `GET /Providers/Subtitles/Subtitles/{subtitleId}` (`GetRemoteSubtitles`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_remote_subtitle(
        &self,
        subtitle_id: impl AsRef<str>,
    ) -> Result<reqwest::Response> {
        let req = self.client.request(
            Method::GET,
            &format!("Providers/Subtitles/Subtitles/{}", subtitle_id.as_ref()),
        )?;
        self.client.execute(req).await
    }

    /// Downloads a remote subtitle file into a file.
    pub async fn download_remote_subtitle_to_file(
        &self,
        subtitle_id: impl AsRef<str>,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        let req = self.client.request(
            Method::GET,
            &format!("Providers/Subtitles/Subtitles/{}", subtitle_id.as_ref()),
        )?;
        self.client.download(req, file_path).await
    }

    /// Uploads an external subtitle file.
    ///
    /// OpenAPI: `POST /Videos/{itemId}/Subtitles` (`UploadSubtitle`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn upload_subtitle(
        &self,
        item_id: uuid::Uuid,
        request: UploadSubtitleRequest,
    ) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, &format!("Videos/{item_id}/Subtitles"))?
            .json(&request);
        self.client.send_unit(req).await
    }

    /// Uploads an external subtitle file from disk.
    ///
    /// The file bytes are base64-encoded as required by Jellyfin's API.
    pub async fn upload_subtitle_from_file(
        &self,
        item_id: uuid::Uuid,
        language: impl Into<String>,
        format: impl Into<String>,
        is_forced: bool,
        is_hearing_impaired: bool,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        let bytes = std::fs::read(file_path)?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let request = UploadSubtitleRequest::new_base64(language, format, encoded)
            .is_forced(is_forced)
            .is_hearing_impaired(is_hearing_impaired);
        self.upload_subtitle(item_id, request).await
    }

    /// Deletes an external subtitle file.
    ///
    /// OpenAPI: `DELETE /Videos/{itemId}/Subtitles/{index}` (`DeleteSubtitle`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn delete_subtitle(&self, item_id: uuid::Uuid, index: i32) -> Result<()> {
        let req = self.client.request(
            Method::DELETE,
            &format!("Videos/{item_id}/Subtitles/{index}"),
        )?;
        self.client.send_unit(req).await
    }

    /// Gets subtitles in a specified format.
    ///
    /// OpenAPI: `GET /Videos/{routeItemId}/{routeMediaSourceId}/Subtitles/{routeIndex}/Stream.{routeFormat}`
    /// (`GetSubtitle`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_subtitle(
        &self,
        item_id: uuid::Uuid,
        media_source_id: impl AsRef<str>,
        subtitle_index: i32,
        format: SubtitleFormat,
        query: SubtitleStreamQuery,
    ) -> Result<reqwest::Response> {
        let path = format!(
            "Videos/{item_id}/{}/Subtitles/{subtitle_index}/Stream.{}",
            media_source_id.as_ref(),
            format.as_str()
        );
        let req = self
            .client
            .request(Method::GET, &path)?
            .query(&query.to_params());
        self.client.execute(req).await
    }

    /// Gets subtitles starting at a specified time in ticks.
    ///
    /// OpenAPI: `GET /Videos/{routeItemId}/{routeMediaSourceId}/Subtitles/{routeIndex}/{routeStartPositionTicks}/Stream.{routeFormat}`
    /// (`GetSubtitleWithTicks`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_subtitle_with_ticks(
        &self,
        item_id: uuid::Uuid,
        media_source_id: impl AsRef<str>,
        subtitle_index: i32,
        start_position_ticks: i64,
        format: SubtitleFormat,
        query: SubtitleStreamQuery,
    ) -> Result<reqwest::Response> {
        let path = format!(
            "Videos/{item_id}/{}/Subtitles/{subtitle_index}/{start_position_ticks}/Stream.{}",
            media_source_id.as_ref(),
            format.as_str()
        );
        let req = self
            .client
            .request(Method::GET, &path)?
            .query(&query.to_params());
        self.client.execute(req).await
    }

    /// Gets an HLS subtitle playlist.
    ///
    /// OpenAPI: `GET /Videos/{itemId}/{mediaSourceId}/Subtitles/{index}/subtitles.m3u8` (`GetSubtitlePlaylist`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_subtitle_playlist(
        &self,
        item_id: uuid::Uuid,
        media_source_id: impl AsRef<str>,
        subtitle_index: i32,
        segment_length: i32,
    ) -> Result<reqwest::Response> {
        let path = format!(
            "Videos/{item_id}/{}/Subtitles/{subtitle_index}/subtitles.m3u8",
            media_source_id.as_ref()
        );
        let req = self
            .client
            .request(Method::GET, &path)?
            .query(&[("segmentLength", segment_length.to_string())]);
        self.client.execute(req).await
    }

    /// Downloads subtitles into a file.
    pub async fn download_subtitle_to_file(
        &self,
        item_id: uuid::Uuid,
        media_source_id: impl AsRef<str>,
        subtitle_index: i32,
        format: SubtitleFormat,
        query: SubtitleStreamQuery,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        let path = format!(
            "Videos/{item_id}/{}/Subtitles/{subtitle_index}/Stream.{}",
            media_source_id.as_ref(),
            format.as_str()
        );
        let req = self
            .client
            .request(Method::GET, &path)?
            .query(&query.to_params());
        self.client.download(req, file_path).await
    }

    /// Downloads subtitles using playback info as guidance.
    ///
    /// This uses the first media source from `PlaybackInfoResponse`.
    pub async fn download_subtitle_from_playback_info_to_file(
        &self,
        item_id: uuid::Uuid,
        playback: &PlaybackInfoResponseStub,
        subtitle_index: i32,
        format: SubtitleFormat,
        query: SubtitleStreamQuery,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        let Some(media_source_id) = playback.media_sources.first().and_then(|s| s.id.clone())
        else {
            return Err(crate::Error::InvalidConfig(
                "playback info contains no media sources",
            ));
        };

        self.download_subtitle_to_file(
            item_id,
            media_source_id,
            subtitle_index,
            format,
            query,
            file_path,
        )
        .await
    }
}
