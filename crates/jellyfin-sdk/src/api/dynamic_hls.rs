use reqwest::Method;

use crate::{JellyfinClient, Result};

/// Common query parameters for Dynamic HLS playlist endpoints.
#[derive(Clone, Debug, Default)]
pub struct HlsPlaylistQuery {
    params: Vec<(String, String)>,
    static_stream: Option<bool>,
    stream_params: Option<String>,
    tag: Option<String>,
    play_session_id: Option<String>,
    segment_container: Option<String>,
    segment_length: Option<i32>,
    min_segments: Option<i32>,
    media_source_id: Option<String>,
    device_id: Option<String>,
    audio_codec: Option<String>,
    video_codec: Option<String>,
    max_streaming_bitrate: Option<i32>,
    start_time_ticks: Option<i64>,
    audio_stream_index: Option<i32>,
    subtitle_stream_index: Option<i32>,
    enable_auto_stream_copy: Option<bool>,
    allow_video_stream_copy: Option<bool>,
    allow_audio_stream_copy: Option<bool>,
}

impl HlsPlaylistQuery {
    /// Creates an empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// If true, the original file will be streamed statically without any encoding.
    pub fn static_stream(mut self, enabled: bool) -> Self {
        self.static_stream = Some(enabled);
        self
    }

    /// The streaming parameters (server-specific).
    pub fn stream_params(mut self, params: impl Into<String>) -> Self {
        self.stream_params = Some(params.into());
        self
    }

    /// The playlist tag.
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// The play session id.
    pub fn play_session_id(mut self, id: impl Into<String>) -> Self {
        self.play_session_id = Some(id.into());
        self
    }

    /// The segment container (e.g. "ts").
    pub fn segment_container(mut self, container: impl Into<String>) -> Self {
        self.segment_container = Some(container.into());
        self
    }

    /// The desired segment length (seconds).
    pub fn segment_length(mut self, seconds: i32) -> Self {
        self.segment_length = Some(seconds);
        self
    }

    /// The minimum number of segments.
    pub fn min_segments(mut self, n: i32) -> Self {
        self.min_segments = Some(n);
        self
    }

    /// The media version id, if playing an alternate version.
    pub fn media_source_id(mut self, id: impl Into<String>) -> Self {
        self.media_source_id = Some(id.into());
        self
    }

    /// The device id of the client requesting.
    pub fn device_id(mut self, id: impl Into<String>) -> Self {
        self.device_id = Some(id.into());
        self
    }

    /// Audio codec to encode to (e.g. "aac", "mp3").
    pub fn audio_codec(mut self, codec: impl Into<String>) -> Self {
        self.audio_codec = Some(codec.into());
        self
    }

    /// Video codec to encode to (e.g. "h264", "hevc").
    pub fn video_codec(mut self, codec: impl Into<String>) -> Self {
        self.video_codec = Some(codec.into());
        self
    }

    /// Max streaming bitrate.
    pub fn max_streaming_bitrate(mut self, bitrate: i32) -> Self {
        self.max_streaming_bitrate = Some(bitrate);
        self
    }

    /// Start time in ticks (100ns).
    pub fn start_time_ticks(mut self, ticks: i64) -> Self {
        self.start_time_ticks = Some(ticks);
        self
    }

    /// Audio stream index.
    pub fn audio_stream_index(mut self, index: i32) -> Self {
        self.audio_stream_index = Some(index);
        self
    }

    /// Subtitle stream index.
    pub fn subtitle_stream_index(mut self, index: i32) -> Self {
        self.subtitle_stream_index = Some(index);
        self
    }

    /// Whether or not to allow automatic stream copy if requested values match the original source.
    pub fn enable_auto_stream_copy(mut self, enabled: bool) -> Self {
        self.enable_auto_stream_copy = Some(enabled);
        self
    }

    /// Whether or not to allow copying of the video stream url.
    pub fn allow_video_stream_copy(mut self, enabled: bool) -> Self {
        self.allow_video_stream_copy = Some(enabled);
        self
    }

    /// Whether or not to allow copying of the audio stream url.
    pub fn allow_audio_stream_copy(mut self, enabled: bool) -> Self {
        self.allow_audio_stream_copy = Some(enabled);
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }

    fn to_params(&self) -> Vec<(String, String)> {
        let mut q = self.params.clone();

        if let Some(v) = self.static_stream {
            q.push(("static".to_owned(), v.to_string()));
        }
        if let Some(v) = &self.stream_params {
            q.push(("params".to_owned(), v.clone()));
        }
        if let Some(v) = &self.tag {
            q.push(("tag".to_owned(), v.clone()));
        }
        if let Some(v) = &self.play_session_id {
            q.push(("playSessionId".to_owned(), v.clone()));
        }
        if let Some(v) = &self.segment_container {
            q.push(("segmentContainer".to_owned(), v.clone()));
        }
        if let Some(v) = self.segment_length {
            q.push(("segmentLength".to_owned(), v.to_string()));
        }
        if let Some(v) = self.min_segments {
            q.push(("minSegments".to_owned(), v.to_string()));
        }
        if let Some(v) = &self.media_source_id {
            q.push(("mediaSourceId".to_owned(), v.clone()));
        }
        if let Some(v) = &self.device_id {
            q.push(("deviceId".to_owned(), v.clone()));
        }
        if let Some(v) = &self.audio_codec {
            q.push(("audioCodec".to_owned(), v.clone()));
        }
        if let Some(v) = &self.video_codec {
            q.push(("videoCodec".to_owned(), v.clone()));
        }
        if let Some(v) = self.max_streaming_bitrate {
            q.push(("maxStreamingBitrate".to_owned(), v.to_string()));
        }
        if let Some(v) = self.start_time_ticks {
            q.push(("startTimeTicks".to_owned(), v.to_string()));
        }
        if let Some(v) = self.audio_stream_index {
            q.push(("audioStreamIndex".to_owned(), v.to_string()));
        }
        if let Some(v) = self.subtitle_stream_index {
            q.push(("subtitleStreamIndex".to_owned(), v.to_string()));
        }
        if let Some(v) = self.enable_auto_stream_copy {
            q.push(("enableAutoStreamCopy".to_owned(), v.to_string()));
        }
        if let Some(v) = self.allow_video_stream_copy {
            q.push(("allowVideoStreamCopy".to_owned(), v.to_string()));
        }
        if let Some(v) = self.allow_audio_stream_copy {
            q.push(("allowAudioStreamCopy".to_owned(), v.to_string()));
        }

        q
    }
}

/// Query parameters for Dynamic HLS segment endpoints.
#[derive(Clone, Debug, Default)]
pub struct HlsSegmentQuery {
    params: Vec<(String, String)>,
    runtime_ticks: i64,
    actual_segment_length_ticks: i64,
    media_source_id: Option<String>,
    device_id: Option<String>,
    play_session_id: Option<String>,
    static_stream: Option<bool>,
    stream_params: Option<String>,
}

impl HlsSegmentQuery {
    /// Creates a query for a specific segment.
    pub fn new(runtime_ticks: i64, actual_segment_length_ticks: i64) -> Self {
        Self {
            runtime_ticks,
            actual_segment_length_ticks,
            ..Default::default()
        }
    }

    /// The media version id, if playing an alternate version.
    pub fn media_source_id(mut self, id: impl Into<String>) -> Self {
        self.media_source_id = Some(id.into());
        self
    }

    /// The device id of the client requesting.
    pub fn device_id(mut self, id: impl Into<String>) -> Self {
        self.device_id = Some(id.into());
        self
    }

    /// The play session id.
    pub fn play_session_id(mut self, id: impl Into<String>) -> Self {
        self.play_session_id = Some(id.into());
        self
    }

    /// If true, the original file will be streamed statically without any encoding.
    pub fn static_stream(mut self, enabled: bool) -> Self {
        self.static_stream = Some(enabled);
        self
    }

    /// The streaming parameters (server-specific).
    pub fn stream_params(mut self, params: impl Into<String>) -> Self {
        self.stream_params = Some(params.into());
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }

    fn to_params(&self) -> Vec<(String, String)> {
        let mut q = self.params.clone();
        q.push(("runtimeTicks".to_owned(), self.runtime_ticks.to_string()));
        q.push((
            "actualSegmentLengthTicks".to_owned(),
            self.actual_segment_length_ticks.to_string(),
        ));
        if let Some(v) = self.static_stream {
            q.push(("static".to_owned(), v.to_string()));
        }
        if let Some(v) = &self.stream_params {
            q.push(("params".to_owned(), v.clone()));
        }
        if let Some(v) = &self.play_session_id {
            q.push(("playSessionId".to_owned(), v.clone()));
        }
        if let Some(v) = &self.media_source_id {
            q.push(("mediaSourceId".to_owned(), v.clone()));
        }
        if let Some(v) = &self.device_id {
            q.push(("deviceId".to_owned(), v.clone()));
        }
        q
    }
}

/// Dynamic HLS (m3u8 playlists and segments) endpoints.
#[derive(Clone, Debug)]
pub struct DynamicHlsApi {
    client: JellyfinClient,
}

impl DynamicHlsApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Gets a video HLS master playlist.
    ///
    /// OpenAPI: `GET /Videos/{itemId}/master.m3u8` (`GetMasterHlsVideoPlaylist`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_master_video_playlist(
        &self,
        item_id: uuid::Uuid,
        media_source_id: impl Into<String>,
        query: HlsPlaylistQuery,
    ) -> Result<reqwest::Response> {
        let query = query.media_source_id(media_source_id);
        let req = self
            .client
            .request(Method::GET, &format!("Videos/{item_id}/master.m3u8"))?
            .query(&query.to_params());
        self.client.execute(req).await
    }

    /// Sends a HEAD request for a video HLS master playlist.
    ///
    /// OpenAPI: `HEAD /Videos/{itemId}/master.m3u8` (`HeadMasterHlsVideoPlaylist`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn head_master_video_playlist(
        &self,
        item_id: uuid::Uuid,
        media_source_id: impl Into<String>,
        query: HlsPlaylistQuery,
    ) -> Result<reqwest::Response> {
        let query = query.media_source_id(media_source_id);
        let req = self
            .client
            .request(Method::HEAD, &format!("Videos/{item_id}/master.m3u8"))?
            .query(&query.to_params());
        self.client.execute(req).await
    }

    /// Gets a video HLS variant playlist.
    ///
    /// OpenAPI: `GET /Videos/{itemId}/main.m3u8` (`GetVariantHlsVideoPlaylist`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_variant_video_playlist(
        &self,
        item_id: uuid::Uuid,
        query: HlsPlaylistQuery,
    ) -> Result<reqwest::Response> {
        let req = self
            .client
            .request(Method::GET, &format!("Videos/{item_id}/main.m3u8"))?
            .query(&query.to_params());
        self.client.execute(req).await
    }

    /// Gets a video HLS segment.
    ///
    /// OpenAPI: `GET /Videos/{itemId}/hls1/{playlistId}/{segmentId}.{container}` (`GetHlsVideoSegment`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_video_segment(
        &self,
        item_id: uuid::Uuid,
        playlist_id: impl AsRef<str>,
        segment_id: i32,
        container: impl AsRef<str>,
        query: HlsSegmentQuery,
    ) -> Result<reqwest::Response> {
        let req = self
            .client
            .request(
                Method::GET,
                &format!(
                    "Videos/{item_id}/hls1/{}/{segment_id}.{}",
                    playlist_id.as_ref(),
                    container.as_ref()
                ),
            )?
            .query(&query.to_params());
        self.client.execute(req).await
    }

    /// Gets a live HLS stream playlist.
    ///
    /// OpenAPI: `GET /Videos/{itemId}/live.m3u8` (`GetLiveHlsStream`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_live_video_playlist(
        &self,
        item_id: uuid::Uuid,
        query: HlsPlaylistQuery,
    ) -> Result<reqwest::Response> {
        let req = self
            .client
            .request(Method::GET, &format!("Videos/{item_id}/live.m3u8"))?
            .query(&query.to_params());
        self.client.execute(req).await
    }

    /// Gets an audio HLS master playlist.
    ///
    /// OpenAPI: `GET /Audio/{itemId}/master.m3u8` (`GetMasterHlsAudioPlaylist`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_master_audio_playlist(
        &self,
        item_id: uuid::Uuid,
        media_source_id: impl Into<String>,
        query: HlsPlaylistQuery,
    ) -> Result<reqwest::Response> {
        let query = query.media_source_id(media_source_id);
        let req = self
            .client
            .request(Method::GET, &format!("Audio/{item_id}/master.m3u8"))?
            .query(&query.to_params());
        self.client.execute(req).await
    }

    /// Sends a HEAD request for an audio HLS master playlist.
    ///
    /// OpenAPI: `HEAD /Audio/{itemId}/master.m3u8` (`HeadMasterHlsAudioPlaylist`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn head_master_audio_playlist(
        &self,
        item_id: uuid::Uuid,
        media_source_id: impl Into<String>,
        query: HlsPlaylistQuery,
    ) -> Result<reqwest::Response> {
        let query = query.media_source_id(media_source_id);
        let req = self
            .client
            .request(Method::HEAD, &format!("Audio/{item_id}/master.m3u8"))?
            .query(&query.to_params());
        self.client.execute(req).await
    }

    /// Gets an audio HLS variant playlist.
    ///
    /// OpenAPI: `GET /Audio/{itemId}/main.m3u8` (`GetVariantHlsAudioPlaylist`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_variant_audio_playlist(
        &self,
        item_id: uuid::Uuid,
        query: HlsPlaylistQuery,
    ) -> Result<reqwest::Response> {
        let req = self
            .client
            .request(Method::GET, &format!("Audio/{item_id}/main.m3u8"))?
            .query(&query.to_params());
        self.client.execute(req).await
    }

    /// Gets an audio HLS segment.
    ///
    /// OpenAPI: `GET /Audio/{itemId}/hls1/{playlistId}/{segmentId}.{container}` (`GetHlsAudioSegment`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_audio_segment(
        &self,
        item_id: uuid::Uuid,
        playlist_id: impl AsRef<str>,
        segment_id: i32,
        container: impl AsRef<str>,
        query: HlsSegmentQuery,
    ) -> Result<reqwest::Response> {
        let req = self
            .client
            .request(
                Method::GET,
                &format!(
                    "Audio/{item_id}/hls1/{}/{segment_id}.{}",
                    playlist_id.as_ref(),
                    container.as_ref()
                ),
            )?
            .query(&query.to_params());
        self.client.execute(req).await
    }
}
