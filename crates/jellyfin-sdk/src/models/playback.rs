use serde::{Deserialize, Serialize};

/// Error codes returned by `PlaybackInfoResponse`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PlaybackErrorCode {
    /// Playback is not allowed.
    NotAllowed,
    /// No compatible stream available.
    NoCompatibleStream,
    /// Rate limit exceeded.
    RateLimitExceeded,
}

/// A minimal subset of `MediaSourceInfo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaSourceInfoStub {
    /// Media source id.
    pub id: Option<String>,
    /// Media container (e.g. "mkv", "mp4").
    pub container: Option<String>,
    /// Media path.
    pub path: Option<String>,
    /// Total size in bytes.
    pub size: Option<i64>,
    /// Bitrate.
    pub bitrate: Option<i32>,
    /// Run time in ticks (100ns).
    pub run_time_ticks: Option<i64>,
    /// Whether direct play is supported.
    pub supports_direct_play: Option<bool>,
    /// Whether direct stream is supported.
    pub supports_direct_stream: Option<bool>,
    /// Whether transcoding is supported.
    pub supports_transcoding: Option<bool>,
    /// Transcoding URL, if any.
    pub transcoding_url: Option<String>,
}

/// A minimal subset of Jellyfin `PlaybackInfoResponse`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackInfoResponseStub {
    /// Media sources for this item.
    #[serde(default)]
    pub media_sources: Vec<MediaSourceInfoStub>,
    /// Play session id.
    pub play_session_id: Option<String>,
    /// Error code, if any.
    pub error_code: Option<PlaybackErrorCode>,
}

/// A minimal subset of Jellyfin `PlaybackInfoDto`.
///
/// Note: Jellyfin supports passing these values via query for backwards compatibility,
/// but new clients should prefer the request body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackInfoRequest {
    /// The playback user id.
    pub user_id: Option<uuid::Uuid>,
    /// The max streaming bitrate.
    pub max_streaming_bitrate: Option<i32>,
    /// The start time in ticks.
    pub start_time_ticks: Option<i64>,
    /// The audio stream index.
    pub audio_stream_index: Option<i32>,
    /// The subtitle stream index.
    pub subtitle_stream_index: Option<i32>,
    /// The max audio channels.
    pub max_audio_channels: Option<i32>,
    /// The media source id.
    pub media_source_id: Option<String>,
    /// The live stream id.
    pub live_stream_id: Option<String>,
    /// Whether to enable direct play.
    pub enable_direct_play: Option<bool>,
    /// Whether to enable direct stream.
    pub enable_direct_stream: Option<bool>,
    /// Whether to enable transcoding.
    pub enable_transcoding: Option<bool>,
    /// Whether to allow to copy the video stream.
    pub allow_video_stream_copy: Option<bool>,
    /// Whether to allow audio stream copy.
    pub allow_audio_stream_copy: Option<bool>,
    /// Whether to auto open the live stream.
    pub auto_open_live_stream: Option<bool>,
    /// Whether to always burn in subtitles when transcoding.
    pub always_burn_in_subtitle_when_transcoding: Option<bool>,
}

impl PlaybackInfoRequest {
    /// Creates an empty request.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the user id.
    pub fn user_id(mut self, user_id: uuid::Uuid) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// Sets max streaming bitrate.
    pub fn max_streaming_bitrate(mut self, bitrate: i32) -> Self {
        self.max_streaming_bitrate = Some(bitrate);
        self
    }

    /// Sets start time ticks.
    pub fn start_time_ticks(mut self, ticks: i64) -> Self {
        self.start_time_ticks = Some(ticks);
        self
    }

    /// Sets audio stream index.
    pub fn audio_stream_index(mut self, index: i32) -> Self {
        self.audio_stream_index = Some(index);
        self
    }

    /// Sets subtitle stream index.
    pub fn subtitle_stream_index(mut self, index: i32) -> Self {
        self.subtitle_stream_index = Some(index);
        self
    }

    /// Sets max audio channels.
    pub fn max_audio_channels(mut self, channels: i32) -> Self {
        self.max_audio_channels = Some(channels);
        self
    }

    /// Sets media source id.
    pub fn media_source_id(mut self, media_source_id: impl Into<String>) -> Self {
        self.media_source_id = Some(media_source_id.into());
        self
    }

    /// Sets live stream id.
    pub fn live_stream_id(mut self, live_stream_id: impl Into<String>) -> Self {
        self.live_stream_id = Some(live_stream_id.into());
        self
    }

    /// Sets direct play enable flag.
    pub fn enable_direct_play(mut self, enable: bool) -> Self {
        self.enable_direct_play = Some(enable);
        self
    }

    /// Sets direct stream enable flag.
    pub fn enable_direct_stream(mut self, enable: bool) -> Self {
        self.enable_direct_stream = Some(enable);
        self
    }

    /// Sets transcoding enable flag.
    pub fn enable_transcoding(mut self, enable: bool) -> Self {
        self.enable_transcoding = Some(enable);
        self
    }

    /// Sets video stream copy enable flag.
    pub fn allow_video_stream_copy(mut self, allow: bool) -> Self {
        self.allow_video_stream_copy = Some(allow);
        self
    }

    /// Sets audio stream copy enable flag.
    pub fn allow_audio_stream_copy(mut self, allow: bool) -> Self {
        self.allow_audio_stream_copy = Some(allow);
        self
    }

    /// Sets auto-open live stream flag.
    pub fn auto_open_live_stream(mut self, enable: bool) -> Self {
        self.auto_open_live_stream = Some(enable);
        self
    }

    /// Sets always-burn-in-subtitle flag when transcoding.
    pub fn always_burn_in_subtitle_when_transcoding(mut self, enable: bool) -> Self {
        self.always_burn_in_subtitle_when_transcoding = Some(enable);
        self
    }
}
