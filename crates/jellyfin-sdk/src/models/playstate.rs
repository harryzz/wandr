use serde::{Deserialize, Serialize};

/// How media is being played.
///
/// OpenAPI: `PlayMethod`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PlayMethod {
    /// Playback is transcoded.
    Transcode,
    /// Playback is direct streamed.
    DirectStream,
    /// Playback is direct played.
    DirectPlay,
}

impl PlayMethod {
    /// Returns the wire representation used by Jellyfin.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transcode => "Transcode",
            Self::DirectStream => "DirectStream",
            Self::DirectPlay => "DirectPlay",
        }
    }
}

impl std::fmt::Display for PlayMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Playback start info.
///
/// OpenAPI: `PlaybackStartInfo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackStart {
    /// The item identifier.
    pub item_id: uuid::Uuid,
    /// Indicates if the client can seek.
    #[serde(default)]
    pub can_seek: bool,
    /// Session id.
    pub session_id: Option<String>,
    /// Media source id.
    pub media_source_id: Option<String>,
    /// Audio stream index.
    pub audio_stream_index: Option<i32>,
    /// Subtitle stream index.
    pub subtitle_stream_index: Option<i32>,
    /// Whether playback is paused.
    #[serde(default)]
    pub is_paused: bool,
    /// Whether playback is muted.
    #[serde(default)]
    pub is_muted: bool,
    /// Current position in ticks (100ns).
    pub position_ticks: Option<i64>,
    /// Playback start time in ticks (100ns).
    pub playback_start_time_ticks: Option<i64>,
    /// Volume level.
    pub volume_level: Option<i32>,
    /// Play method.
    pub play_method: Option<PlayMethod>,
    /// Live stream id.
    pub live_stream_id: Option<String>,
    /// Play session id.
    pub play_session_id: Option<String>,
}

impl PlaybackStart {
    /// Creates a minimal playback start payload.
    pub fn new(item_id: uuid::Uuid) -> Self {
        Self {
            item_id,
            can_seek: true,
            session_id: None,
            media_source_id: None,
            audio_stream_index: None,
            subtitle_stream_index: None,
            is_paused: false,
            is_muted: false,
            position_ticks: None,
            playback_start_time_ticks: None,
            volume_level: None,
            play_method: None,
            live_stream_id: None,
            play_session_id: None,
        }
    }

    /// Sets the media source id.
    pub fn media_source_id(mut self, id: impl Into<String>) -> Self {
        self.media_source_id = Some(id.into());
        self
    }

    /// Sets the play session id.
    pub fn play_session_id(mut self, id: impl Into<String>) -> Self {
        self.play_session_id = Some(id.into());
        self
    }

    /// Sets the current position ticks.
    pub fn position_ticks(mut self, ticks: i64) -> Self {
        self.position_ticks = Some(ticks);
        self
    }
}

/// Playback progress info.
///
/// OpenAPI: `PlaybackProgressInfo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackProgress {
    /// The item identifier.
    pub item_id: uuid::Uuid,
    /// Indicates if the client can seek.
    #[serde(default)]
    pub can_seek: bool,
    /// Session id.
    pub session_id: Option<String>,
    /// Media source id.
    pub media_source_id: Option<String>,
    /// Audio stream index.
    pub audio_stream_index: Option<i32>,
    /// Subtitle stream index.
    pub subtitle_stream_index: Option<i32>,
    /// Whether playback is paused.
    #[serde(default)]
    pub is_paused: bool,
    /// Whether playback is muted.
    #[serde(default)]
    pub is_muted: bool,
    /// Current position in ticks (100ns).
    pub position_ticks: Option<i64>,
    /// Playback start time in ticks (100ns).
    pub playback_start_time_ticks: Option<i64>,
    /// Volume level.
    pub volume_level: Option<i32>,
    /// Play method.
    pub play_method: Option<PlayMethod>,
    /// Live stream id.
    pub live_stream_id: Option<String>,
    /// Play session id.
    pub play_session_id: Option<String>,
}

impl PlaybackProgress {
    /// Creates a minimal playback progress payload.
    pub fn new(item_id: uuid::Uuid) -> Self {
        Self {
            item_id,
            can_seek: true,
            session_id: None,
            media_source_id: None,
            audio_stream_index: None,
            subtitle_stream_index: None,
            is_paused: false,
            is_muted: false,
            position_ticks: None,
            playback_start_time_ticks: None,
            volume_level: None,
            play_method: None,
            live_stream_id: None,
            play_session_id: None,
        }
    }

    /// Sets the media source id.
    pub fn media_source_id(mut self, id: impl Into<String>) -> Self {
        self.media_source_id = Some(id.into());
        self
    }

    /// Sets the play session id.
    pub fn play_session_id(mut self, id: impl Into<String>) -> Self {
        self.play_session_id = Some(id.into());
        self
    }

    /// Sets the current position ticks.
    pub fn position_ticks(mut self, ticks: i64) -> Self {
        self.position_ticks = Some(ticks);
        self
    }
}

/// Playback stop info.
///
/// OpenAPI: `PlaybackStopInfo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackStop {
    /// The item identifier.
    pub item_id: uuid::Uuid,
    /// Session id.
    pub session_id: Option<String>,
    /// Media source id.
    pub media_source_id: Option<String>,
    /// Current position in ticks (100ns).
    pub position_ticks: Option<i64>,
    /// Live stream id.
    pub live_stream_id: Option<String>,
    /// Play session id.
    pub play_session_id: Option<String>,
    /// Whether playback failed.
    #[serde(default)]
    pub failed: bool,
}

impl PlaybackStop {
    /// Creates a minimal playback stop payload.
    pub fn new(item_id: uuid::Uuid) -> Self {
        Self {
            item_id,
            session_id: None,
            media_source_id: None,
            position_ticks: None,
            live_stream_id: None,
            play_session_id: None,
            failed: false,
        }
    }

    /// Sets the media source id.
    pub fn media_source_id(mut self, id: impl Into<String>) -> Self {
        self.media_source_id = Some(id.into());
        self
    }

    /// Sets the play session id.
    pub fn play_session_id(mut self, id: impl Into<String>) -> Self {
        self.play_session_id = Some(id.into());
        self
    }

    /// Sets the current position ticks.
    pub fn position_ticks(mut self, ticks: i64) -> Self {
        self.position_ticks = Some(ticks);
        self
    }
}
