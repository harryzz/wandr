use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{PlayCommand, PlaystateCommand, SessionInfoStub},
};

/// Query parameters for `GET /Sessions`.
#[derive(Clone, Debug, Default)]
pub struct SessionsQuery {
    params: Vec<(String, String)>,
}

impl SessionsQuery {
    /// Creates an empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter by sessions that a given user is allowed to remote control.
    pub fn controllable_by_user_id(mut self, user_id: uuid::Uuid) -> Self {
        self.params
            .push(("controllableByUserId".to_owned(), user_id.to_string()));
        self
    }

    /// Filter by device id.
    pub fn device_id(mut self, device_id: impl Into<String>) -> Self {
        self.params.push(("deviceId".to_owned(), device_id.into()));
        self
    }

    /// Filter by sessions active within the last N seconds.
    pub fn active_within_seconds(mut self, seconds: u32) -> Self {
        self.params
            .push(("activeWithinSeconds".to_owned(), seconds.to_string()));
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }
}

/// Options for `POST /Sessions/{sessionId}/Playing/{command}`.
#[derive(Clone, Debug, Default)]
pub struct PlaystateOptions {
    seek_position_ticks: Option<i64>,
    controlling_user_id: Option<String>,
}

impl PlaystateOptions {
    /// Creates an empty options set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the seek position in ticks (100ns).
    pub fn seek_position_ticks(mut self, ticks: i64) -> Self {
        self.seek_position_ticks = Some(ticks);
        self
    }

    /// Sets the controlling user id.
    pub fn controlling_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.controlling_user_id = Some(user_id.into());
        self
    }

    fn to_query(&self) -> Vec<(String, String)> {
        let mut q = Vec::new();
        if let Some(v) = self.seek_position_ticks {
            q.push(("seekPositionTicks".to_owned(), v.to_string()));
        }
        if let Some(v) = &self.controlling_user_id {
            q.push(("controllingUserId".to_owned(), v.clone()));
        }
        q
    }
}

/// Options for `POST /Sessions/{sessionId}/Playing`.
#[derive(Clone, Debug)]
pub struct PlayOptions {
    play_command: PlayCommand,
    item_ids: Vec<uuid::Uuid>,
    start_position_ticks: Option<i64>,
    media_source_id: Option<String>,
    audio_stream_index: Option<i32>,
    subtitle_stream_index: Option<i32>,
    start_index: Option<i32>,
}

impl PlayOptions {
    /// Creates a play request.
    pub fn new(play_command: PlayCommand, item_ids: impl Into<Vec<uuid::Uuid>>) -> Self {
        Self {
            play_command,
            item_ids: item_ids.into(),
            start_position_ticks: None,
            media_source_id: None,
            audio_stream_index: None,
            subtitle_stream_index: None,
            start_index: None,
        }
    }

    /// Sets the starting position of the first item (ticks, 100ns).
    pub fn start_position_ticks(mut self, ticks: i64) -> Self {
        self.start_position_ticks = Some(ticks);
        self
    }

    /// Sets the media source id.
    pub fn media_source_id(mut self, id: impl Into<String>) -> Self {
        self.media_source_id = Some(id.into());
        self
    }

    /// Sets the audio stream index.
    pub fn audio_stream_index(mut self, index: i32) -> Self {
        self.audio_stream_index = Some(index);
        self
    }

    /// Sets the subtitle stream index.
    pub fn subtitle_stream_index(mut self, index: i32) -> Self {
        self.subtitle_stream_index = Some(index);
        self
    }

    /// Sets the start index.
    pub fn start_index(mut self, index: i32) -> Self {
        self.start_index = Some(index);
        self
    }

    fn to_query(&self) -> Vec<(String, String)> {
        let mut q = Vec::new();
        q.push(("playCommand".to_owned(), self.play_command.to_string()));

        let joined = self
            .item_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        q.push(("itemIds".to_owned(), joined));

        if let Some(v) = self.start_position_ticks {
            q.push(("startPositionTicks".to_owned(), v.to_string()));
        }
        if let Some(v) = &self.media_source_id {
            q.push(("mediaSourceId".to_owned(), v.clone()));
        }
        if let Some(v) = self.audio_stream_index {
            q.push(("audioStreamIndex".to_owned(), v.to_string()));
        }
        if let Some(v) = self.subtitle_stream_index {
            q.push(("subtitleStreamIndex".to_owned(), v.to_string()));
        }
        if let Some(v) = self.start_index {
            q.push(("startIndex".to_owned(), v.to_string()));
        }

        q
    }
}

/// Session and remote playback control endpoints.
#[derive(Clone, Debug)]
pub struct SessionsApi {
    client: JellyfinClient,
}

/// Controls how a session is selected from a list.
#[derive(Clone, Debug)]
pub struct SessionSelector {
    device_name_contains: Option<String>,
    client_contains: Option<String>,
    prefer_active: bool,
    require_media_control: bool,
    require_remote_control: bool,
}

impl Default for SessionSelector {
    fn default() -> Self {
        Self {
            device_name_contains: None,
            client_contains: None,
            prefer_active: true,
            require_media_control: false,
            require_remote_control: false,
        }
    }
}

impl SessionSelector {
    /// Creates a selector with no filters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Filters sessions whose `device_name` contains the given string (case-insensitive).
    pub fn device_name_contains(mut self, value: impl Into<String>) -> Self {
        self.device_name_contains = Some(value.into());
        self
    }

    /// Filters sessions whose `client` contains the given string (case-insensitive).
    pub fn client_contains(mut self, value: impl Into<String>) -> Self {
        self.client_contains = Some(value.into());
        self
    }

    /// Prefer active sessions when multiple match.
    pub fn prefer_active(mut self, prefer: bool) -> Self {
        self.prefer_active = prefer;
        self
    }

    /// Require `supports_media_control == true`.
    pub fn require_media_control(mut self, require: bool) -> Self {
        self.require_media_control = require;
        self
    }

    /// Require `supports_remote_control == true`.
    pub fn require_remote_control(mut self, require: bool) -> Self {
        self.require_remote_control = require;
        self
    }
}

/// A bound remote session handle (ergonomic wrapper around a `session_id`).
#[derive(Clone, Debug)]
pub struct RemoteSession {
    api: SessionsApi,
    session_id: String,
}

impl SessionsApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Creates a bound remote session handle.
    pub fn remote(&self, session_id: impl Into<String>) -> RemoteSession {
        RemoteSession {
            api: self.clone(),
            session_id: session_id.into(),
        }
    }

    /// Gets a list of sessions.
    ///
    /// OpenAPI: `GET /Sessions` (`GetSessions`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_sessions(&self, query: SessionsQuery) -> Result<Vec<SessionInfoStub>> {
        let req = self
            .client
            .request(Method::GET, "Sessions")?
            .query(&query.params);
        self.client.send_json(req).await
    }

    /// Gets sessions that the specified user is allowed to remote control.
    pub async fn controllable_by_user(&self, user_id: uuid::Uuid) -> Result<Vec<SessionInfoStub>> {
        self.get_sessions(SessionsQuery::new().controllable_by_user_id(user_id))
            .await
    }

    /// Selects the first controllable session for a user, preferring active sessions that support media control.
    pub async fn first_controllable_session(
        &self,
        user_id: uuid::Uuid,
    ) -> Result<Option<RemoteSession>> {
        self.first_controllable_session_matching(user_id, SessionSelector::new())
            .await
    }

    /// Selects the first controllable session for a user, applying additional client-side matching.
    pub async fn first_controllable_session_matching(
        &self,
        user_id: uuid::Uuid,
        selector: SessionSelector,
    ) -> Result<Option<RemoteSession>> {
        let sessions = self.controllable_by_user(user_id).await?;
        Ok(select_preferred_session(sessions, &selector).map(|id| self.remote(id)))
    }

    /// Finds the session(s) for this client's `device_id`, if any.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn my_sessions(&self) -> Result<Vec<SessionInfoStub>> {
        let query = SessionsQuery::new().device_id(self.client.device_id().to_owned());
        self.get_sessions(query).await
    }

    /// Sends a playstate command to a session.
    ///
    /// OpenAPI: `POST /Sessions/{sessionId}/Playing/{command}` (`SendPlaystateCommand`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn send_playstate(
        &self,
        session_id: &str,
        command: PlaystateCommand,
        options: PlaystateOptions,
    ) -> Result<()> {
        let path = format!("Sessions/{session_id}/Playing/{}", command.as_str());
        let req = self
            .client
            .request(Method::POST, &path)?
            .query(&options.to_query());
        self.client.send_unit(req).await
    }

    /// Pauses playback for a session.
    pub async fn pause(&self, session_id: &str) -> Result<()> {
        self.send_playstate(session_id, PlaystateCommand::Pause, PlaystateOptions::new())
            .await
    }

    /// Unpauses playback for a session.
    pub async fn unpause(&self, session_id: &str) -> Result<()> {
        self.send_playstate(
            session_id,
            PlaystateCommand::Unpause,
            PlaystateOptions::new(),
        )
        .await
    }

    /// Toggles play/pause for a session.
    pub async fn play_pause(&self, session_id: &str) -> Result<()> {
        self.send_playstate(
            session_id,
            PlaystateCommand::PlayPause,
            PlaystateOptions::new(),
        )
        .await
    }

    /// Stops playback for a session.
    pub async fn stop(&self, session_id: &str) -> Result<()> {
        self.send_playstate(session_id, PlaystateCommand::Stop, PlaystateOptions::new())
            .await
    }

    /// Skips to the next track for a session.
    pub async fn next_track(&self, session_id: &str) -> Result<()> {
        self.send_playstate(
            session_id,
            PlaystateCommand::NextTrack,
            PlaystateOptions::new(),
        )
        .await
    }

    /// Skips to the previous track for a session.
    pub async fn previous_track(&self, session_id: &str) -> Result<()> {
        self.send_playstate(
            session_id,
            PlaystateCommand::PreviousTrack,
            PlaystateOptions::new(),
        )
        .await
    }

    /// Rewinds for a session.
    pub async fn rewind(&self, session_id: &str) -> Result<()> {
        self.send_playstate(
            session_id,
            PlaystateCommand::Rewind,
            PlaystateOptions::new(),
        )
        .await
    }

    /// Fast-forwards for a session.
    pub async fn fast_forward(&self, session_id: &str) -> Result<()> {
        self.send_playstate(
            session_id,
            PlaystateCommand::FastForward,
            PlaystateOptions::new(),
        )
        .await
    }

    /// Seeks to a position in ticks (100ns).
    pub async fn seek_ticks(&self, session_id: &str, ticks: i64) -> Result<()> {
        self.send_playstate(
            session_id,
            PlaystateCommand::Seek,
            PlaystateOptions::new().seek_position_ticks(ticks),
        )
        .await
    }

    /// Seeks to a position in seconds.
    pub async fn seek_seconds(&self, session_id: &str, seconds: u64) -> Result<()> {
        self.seek_ticks(session_id, seconds_to_ticks(seconds)).await
    }

    /// Seeks to a position in milliseconds.
    pub async fn seek_millis(&self, session_id: &str, millis: u64) -> Result<()> {
        self.seek_ticks(session_id, millis_to_ticks(millis)).await
    }

    /// Seeks to a position represented by a `Duration`.
    pub async fn seek_duration(
        &self,
        session_id: &str,
        duration: std::time::Duration,
    ) -> Result<()> {
        self.seek_ticks(session_id, duration_to_ticks(duration))
            .await
    }

    /// Instructs a session to play one or more items.
    ///
    /// OpenAPI: `POST /Sessions/{sessionId}/Playing` (`Play`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn play(&self, session_id: &str, options: PlayOptions) -> Result<()> {
        let path = format!("Sessions/{session_id}/Playing");
        let req = self
            .client
            .request(Method::POST, &path)?
            .query(&options.to_query());
        self.client.send_unit(req).await
    }
}

impl RemoteSession {
    /// Returns the underlying session id.
    pub fn id(&self) -> &str {
        &self.session_id
    }

    /// Pauses playback.
    pub async fn pause(&self) -> Result<()> {
        self.api.pause(&self.session_id).await
    }

    /// Unpauses playback.
    pub async fn unpause(&self) -> Result<()> {
        self.api.unpause(&self.session_id).await
    }

    /// Toggles play/pause.
    pub async fn play_pause(&self) -> Result<()> {
        self.api.play_pause(&self.session_id).await
    }

    /// Stops playback.
    pub async fn stop(&self) -> Result<()> {
        self.api.stop(&self.session_id).await
    }

    /// Skips to the next track.
    pub async fn next_track(&self) -> Result<()> {
        self.api.next_track(&self.session_id).await
    }

    /// Skips to the previous track.
    pub async fn previous_track(&self) -> Result<()> {
        self.api.previous_track(&self.session_id).await
    }

    /// Rewinds.
    pub async fn rewind(&self) -> Result<()> {
        self.api.rewind(&self.session_id).await
    }

    /// Fast-forwards.
    pub async fn fast_forward(&self) -> Result<()> {
        self.api.fast_forward(&self.session_id).await
    }

    /// Seeks to a position in ticks (100ns).
    pub async fn seek_ticks(&self, ticks: i64) -> Result<()> {
        self.api.seek_ticks(&self.session_id, ticks).await
    }

    /// Seeks to a position in seconds.
    pub async fn seek_seconds(&self, seconds: u64) -> Result<()> {
        self.api.seek_seconds(&self.session_id, seconds).await
    }

    /// Seeks to a position in milliseconds.
    pub async fn seek_millis(&self, millis: u64) -> Result<()> {
        self.api.seek_millis(&self.session_id, millis).await
    }

    /// Seeks to a position represented by a `Duration`.
    pub async fn seek_duration(&self, duration: std::time::Duration) -> Result<()> {
        self.api.seek_duration(&self.session_id, duration).await
    }

    /// Seeks to a position in a timecode string (`SS`, `MM:SS`, `HH:MM:SS`).
    pub async fn seek_timecode(&self, timecode: &str) -> Result<()> {
        let seconds = parse_timecode_seconds(timecode)
            .ok_or_else(|| crate::Error::InvalidTimecode(timecode.to_owned()))?;
        self.seek_seconds(seconds).await
    }

    /// Instructs the session to play one or more items.
    pub async fn play(&self, options: PlayOptions) -> Result<()> {
        self.api.play(&self.session_id, options).await
    }
}

fn seconds_to_ticks(seconds: u64) -> i64 {
    const TICKS_PER_SECOND: i64 = 10_000_000;
    (seconds as i64).saturating_mul(TICKS_PER_SECOND)
}

fn millis_to_ticks(millis: u64) -> i64 {
    const TICKS_PER_MILLI: i64 = 10_000;
    (millis as i64).saturating_mul(TICKS_PER_MILLI)
}

fn duration_to_ticks(duration: std::time::Duration) -> i64 {
    let secs_ticks = seconds_to_ticks(duration.as_secs());
    let nanos = duration.subsec_nanos() as u64;
    let extra_ticks = (nanos / 100) as i64;
    secs_ticks.saturating_add(extra_ticks)
}

fn select_preferred_session(
    sessions: Vec<SessionInfoStub>,
    selector: &SessionSelector,
) -> Option<String> {
    fn score(s: &SessionInfoStub, prefer_active: bool) -> u8 {
        let mut v = 0;
        if prefer_active && s.is_active.unwrap_or(false) {
            v += 4;
        }
        if s.supports_media_control.unwrap_or(false) {
            v += 2;
        }
        if s.supports_remote_control.unwrap_or(false) {
            v += 1;
        }
        v
    }

    sessions
        .into_iter()
        .filter(|s| session_matches_selector(s, selector))
        .filter_map(|s| {
            let sc = score(&s, selector.prefer_active);
            s.id.map(|id| (sc, id))
        })
        .max_by_key(|(s, _id)| *s)
        .map(|(_s, id)| id)
}

fn session_matches_selector(session: &SessionInfoStub, selector: &SessionSelector) -> bool {
    if selector.require_media_control && !session.supports_media_control.unwrap_or(false) {
        return false;
    }
    if selector.require_remote_control && !session.supports_remote_control.unwrap_or(false) {
        return false;
    }

    if let Some(needle) = &selector.device_name_contains {
        let Some(hay) = &session.device_name else {
            return false;
        };
        if !contains_case_insensitive(hay, needle) {
            return false;
        }
    }

    if let Some(needle) = &selector.client_contains {
        let Some(hay) = &session.client else {
            return false;
        };
        if !contains_case_insensitive(hay, needle) {
            return false;
        }
    }

    true
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn parse_timecode_seconds(timecode: &str) -> Option<u64> {
    let t = timecode.trim();
    if t.is_empty() {
        return None;
    }

    let parts: Vec<&str> = t.split(':').collect();
    match parts.len() {
        1 => parts[0].parse::<u64>().ok(),
        2 => {
            let m = parts[0].parse::<u64>().ok()?;
            let s = parts[1].parse::<u64>().ok()?;
            Some(m.saturating_mul(60).saturating_add(s))
        }
        3 => {
            let h = parts[0].parse::<u64>().ok()?;
            let m = parts[1].parse::<u64>().ok()?;
            let s = parts[2].parse::<u64>().ok()?;
            Some(
                h.saturating_mul(3600)
                    .saturating_add(m.saturating_mul(60))
                    .saturating_add(s),
            )
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_conversion_is_100ns() {
        assert_eq!(seconds_to_ticks(0), 0);
        assert_eq!(seconds_to_ticks(1), 10_000_000);
        assert_eq!(seconds_to_ticks(2), 20_000_000);
    }

    #[test]
    fn duration_to_ticks_includes_subsecond() {
        assert_eq!(duration_to_ticks(std::time::Duration::from_millis(0)), 0);
        assert_eq!(
            duration_to_ticks(std::time::Duration::from_millis(1)),
            10_000
        );
        assert_eq!(
            duration_to_ticks(std::time::Duration::from_millis(250)),
            2_500_000
        );
        assert_eq!(
            duration_to_ticks(std::time::Duration::from_secs(1)),
            10_000_000
        );
        assert_eq!(
            duration_to_ticks(
                std::time::Duration::from_secs(1) + std::time::Duration::from_millis(1)
            ),
            10_010_000
        );
    }

    #[test]
    fn millis_to_ticks_is_100ns() {
        assert_eq!(millis_to_ticks(0), 0);
        assert_eq!(millis_to_ticks(1), 10_000);
        assert_eq!(millis_to_ticks(1000), 10_000_000);
    }

    #[test]
    fn selects_active_and_controllable() {
        let a = SessionInfoStub {
            id: Some("a".to_owned()),
            is_active: Some(false),
            supports_media_control: Some(true),
            supports_remote_control: Some(true),
            ..SessionInfoStub::default()
        };

        let b = SessionInfoStub {
            id: Some("b".to_owned()),
            is_active: Some(true),
            supports_media_control: Some(true),
            supports_remote_control: Some(false),
            ..SessionInfoStub::default()
        };

        assert_eq!(
            select_preferred_session(vec![a, b], &SessionSelector::new()),
            Some("b".to_owned())
        );
    }

    #[test]
    fn filters_by_device_name() {
        let a = SessionInfoStub {
            id: Some("a".to_owned()),
            device_name: Some("Living Room TV".to_owned()),
            supports_media_control: Some(true),
            ..SessionInfoStub::default()
        };
        let b = SessionInfoStub {
            id: Some("b".to_owned()),
            device_name: Some("Bedroom".to_owned()),
            supports_media_control: Some(true),
            ..SessionInfoStub::default()
        };

        let selector = SessionSelector::new()
            .device_name_contains("living")
            .require_media_control(true);

        assert_eq!(
            select_preferred_session(vec![a, b], &selector),
            Some("a".to_owned())
        );
    }

    #[test]
    fn parses_timecodes() {
        assert_eq!(parse_timecode_seconds("10"), Some(10));
        assert_eq!(parse_timecode_seconds("01:02"), Some(62));
        assert_eq!(parse_timecode_seconds("1:02:03"), Some(3723));
        assert_eq!(parse_timecode_seconds(""), None);
        assert_eq!(parse_timecode_seconds("1:2:3:4"), None);
        assert_eq!(parse_timecode_seconds("xx"), None);
    }
}
