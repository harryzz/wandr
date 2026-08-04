use serde::{Deserialize, Serialize};

/// Subtitle output formats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubtitleFormat {
    /// SubRip Text.
    Srt,
    /// WebVTT.
    Vtt,
    /// Advanced SubStation Alpha.
    Ass,
    /// SubStation Alpha.
    Ssa,
    /// Other / server-specific format.
    Other(String),
}

impl SubtitleFormat {
    /// Returns the wire representation used by Jellyfin.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Ass => "ass",
            Self::Ssa => "ssa",
            Self::Other(v) => v.as_str(),
        }
    }
}

impl From<&str> for SubtitleFormat {
    fn from(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "srt" => Self::Srt,
            "vtt" => Self::Vtt,
            "ass" => Self::Ass,
            "ssa" => Self::Ssa,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// Query parameters for `GET .../Subtitles/.../Stream.{format}`.
#[derive(Clone, Debug, Default)]
pub struct SubtitleStreamQuery {
    start_position_ticks: Option<i64>,
    end_position_ticks: Option<i64>,
    copy_timestamps: Option<bool>,
    add_vtt_time_map: Option<bool>,
    extra: Vec<(String, String)>,
}

impl SubtitleStreamQuery {
    /// Creates an empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// The start position of the subtitle in ticks.
    pub fn start_position_ticks(mut self, ticks: i64) -> Self {
        self.start_position_ticks = Some(ticks);
        self
    }

    /// Optional. The end position of the subtitle in ticks.
    pub fn end_position_ticks(mut self, ticks: i64) -> Self {
        self.end_position_ticks = Some(ticks);
        self
    }

    /// Optional. Whether to copy the timestamps.
    pub fn copy_timestamps(mut self, value: bool) -> Self {
        self.copy_timestamps = Some(value);
        self
    }

    /// Optional. Whether to add a VTT time map (useful for some players).
    pub fn add_vtt_time_map(mut self, value: bool) -> Self {
        self.add_vtt_time_map = Some(value);
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.push((key.into(), value.into()));
        self
    }

    pub(crate) fn to_params(&self) -> Vec<(String, String)> {
        let mut q = Vec::new();
        if let Some(v) = self.end_position_ticks {
            q.push(("endPositionTicks".to_owned(), v.to_string()));
        }
        if let Some(v) = self.copy_timestamps {
            q.push(("copyTimestamps".to_owned(), v.to_string()));
        }
        if let Some(v) = self.add_vtt_time_map {
            q.push(("addVttTimeMap".to_owned(), v.to_string()));
        }
        if let Some(v) = self.start_position_ticks {
            q.push(("startPositionTicks".to_owned(), v.to_string()));
        }

        q.extend(self.extra.clone());
        q
    }
}

/// Fallback font file information.
///
/// OpenAPI: `FontFile`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FontFile {
    /// File name.
    pub name: Option<String>,
    /// File size in bytes.
    pub size: Option<i64>,
    /// Date created (ISO 8601).
    pub date_created: Option<String>,
    /// Date modified (ISO 8601).
    pub date_modified: Option<String>,
}

/// Remote subtitle search result.
///
/// OpenAPI: `RemoteSubtitleInfo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RemoteSubtitleInfo {
    /// ISO-639-2 language code.
    pub three_letter_iso_language_name: Option<String>,
    /// Remote subtitle id (provider-specific).
    pub id: Option<String>,
    /// Subtitle provider name.
    pub provider_name: Option<String>,
    /// Display name.
    pub name: Option<String>,
    /// Format (e.g. "srt").
    pub format: Option<String>,
    /// Author.
    pub author: Option<String>,
    /// Comment.
    pub comment: Option<String>,
    /// Date created (ISO 8601).
    pub date_created: Option<String>,
    /// Community rating.
    pub community_rating: Option<f32>,
    /// Frame rate.
    pub frame_rate: Option<f32>,
    /// Download count.
    pub download_count: Option<i32>,
    /// Whether this matches by hash.
    pub is_hash_match: Option<bool>,
    /// Whether this is AI translated.
    pub ai_translated: Option<bool>,
    /// Whether this is machine translated.
    pub machine_translated: Option<bool>,
    /// Whether this subtitle is forced.
    pub forced: Option<bool>,
    /// Whether this is hearing impaired.
    pub hearing_impaired: Option<bool>,
}

/// Upload subtitle request body.
///
/// Note: `data` must be base64-encoded subtitle file bytes (as required by Jellyfin).
///
/// OpenAPI: `UploadSubtitleDto`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UploadSubtitleRequest {
    /// Subtitle language (e.g. "eng").
    pub language: String,
    /// Subtitle format (e.g. "srt", "ass").
    pub format: String,
    /// Whether the subtitle is forced.
    pub is_forced: bool,
    /// Whether the subtitle is for hearing impaired.
    pub is_hearing_impaired: bool,
    /// Base64-encoded subtitle file bytes.
    pub data: String,
}

impl UploadSubtitleRequest {
    /// Creates an upload request with base64-encoded data.
    pub fn new_base64(
        language: impl Into<String>,
        format: impl Into<String>,
        data_base64: impl Into<String>,
    ) -> Self {
        Self {
            language: language.into(),
            format: format.into(),
            is_forced: false,
            is_hearing_impaired: false,
            data: data_base64.into(),
        }
    }

    /// Sets the forced flag.
    pub fn is_forced(mut self, is_forced: bool) -> Self {
        self.is_forced = is_forced;
        self
    }

    /// Sets the hearing impaired flag.
    pub fn is_hearing_impaired(mut self, is_hearing_impaired: bool) -> Self {
        self.is_hearing_impaired = is_hearing_impaired;
        self
    }
}
