use reqwest::Method;

use crate::{
    JellyfinClient, Result,
    models::{PlayMethod, PlaybackProgress, PlaybackStart, PlaybackStop, UserItemData},
};

/// Query parameters for `POST /PlayingItems/{itemId}` and related endpoints.
#[derive(Clone, Debug, Default)]
pub struct OnPlaybackQuery {
    params: Vec<(String, String)>,
}

impl OnPlaybackQuery {
    /// Creates an empty query.
    pub fn new() -> Self {
        Self::default()
    }

    /// The media source id.
    pub fn media_source_id(mut self, id: impl Into<String>) -> Self {
        self.params.push(("mediaSourceId".to_owned(), id.into()));
        self
    }

    /// The audio stream index.
    pub fn audio_stream_index(mut self, index: i32) -> Self {
        self.params
            .push(("audioStreamIndex".to_owned(), index.to_string()));
        self
    }

    /// The subtitle stream index.
    pub fn subtitle_stream_index(mut self, index: i32) -> Self {
        self.params
            .push(("subtitleStreamIndex".to_owned(), index.to_string()));
        self
    }

    /// The play method.
    pub fn play_method(mut self, method: PlayMethod) -> Self {
        self.params
            .push(("playMethod".to_owned(), method.to_string()));
        self
    }

    /// The live stream id.
    pub fn live_stream_id(mut self, id: impl Into<String>) -> Self {
        self.params.push(("liveStreamId".to_owned(), id.into()));
        self
    }

    /// The play session id.
    pub fn play_session_id(mut self, id: impl Into<String>) -> Self {
        self.params.push(("playSessionId".to_owned(), id.into()));
        self
    }

    /// Indicates if the client can seek.
    pub fn can_seek(mut self, can_seek: bool) -> Self {
        self.params
            .push(("canSeek".to_owned(), can_seek.to_string()));
        self
    }

    /// The current position in ticks (100ns).
    pub fn position_ticks(mut self, ticks: i64) -> Self {
        self.params
            .push(("positionTicks".to_owned(), ticks.to_string()));
        self
    }

    /// Whether playback is paused.
    pub fn is_paused(mut self, paused: bool) -> Self {
        self.params
            .push(("isPaused".to_owned(), paused.to_string()));
        self
    }

    /// Adds a raw query parameter for forward compatibility.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.push((key.into(), value.into()));
        self
    }

    fn params(&self) -> &[(String, String)] {
        &self.params
    }
}

/// Playstate reporting endpoints.
#[derive(Clone, Debug)]
pub struct PlaystateApi {
    client: JellyfinClient,
}

impl PlaystateApi {
    pub(crate) fn new(client: JellyfinClient) -> Self {
        Self { client }
    }

    /// Reports playback has started within a session.
    ///
    /// OpenAPI: `POST /Sessions/Playing` (`ReportPlaybackStart`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn report_playback_start(&self, info: PlaybackStart) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, "Sessions/Playing")?
            .json(&info);
        self.client.send_unit(req).await
    }

    /// Pings a playback session.
    ///
    /// OpenAPI: `POST /Sessions/Playing/Ping` (`PingPlaybackSession`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn ping_playback_session(&self, play_session_id: impl Into<String>) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, "Sessions/Playing/Ping")?
            .query(&[("playSessionId", play_session_id.into())]);
        self.client.send_unit(req).await
    }

    /// Reports playback progress within a session.
    ///
    /// OpenAPI: `POST /Sessions/Playing/Progress` (`ReportPlaybackProgress`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn report_playback_progress(&self, info: PlaybackProgress) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, "Sessions/Playing/Progress")?
            .json(&info);
        self.client.send_unit(req).await
    }

    /// Reports playback has stopped within a session.
    ///
    /// OpenAPI: `POST /Sessions/Playing/Stopped` (`ReportPlaybackStopped`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn report_playback_stopped(&self, info: PlaybackStop) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, "Sessions/Playing/Stopped")?
            .json(&info);
        self.client.send_unit(req).await
    }

    /// Marks an item as played for a user.
    ///
    /// OpenAPI: `POST /UserPlayedItems/{itemId}` (`MarkPlayedItem`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn mark_played(
        &self,
        item_id: uuid::Uuid,
        user_id: Option<uuid::Uuid>,
        date_played: Option<String>,
    ) -> Result<UserItemData> {
        let mut req = self
            .client
            .request(Method::POST, &format!("UserPlayedItems/{item_id}"))?;
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        if let Some(date_played) = date_played {
            req = req.query(&[("datePlayed", date_played)]);
        }
        self.client.send_json(req).await
    }

    /// Marks an item as unplayed for a user.
    ///
    /// OpenAPI: `DELETE /UserPlayedItems/{itemId}` (`MarkUnplayedItem`).
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn mark_unplayed(
        &self,
        item_id: uuid::Uuid,
        user_id: Option<uuid::Uuid>,
    ) -> Result<UserItemData> {
        let mut req = self
            .client
            .request(Method::DELETE, &format!("UserPlayedItems/{item_id}"))?;
        if let Some(user_id) = user_id {
            req = req.query(&[("userId", user_id.to_string())]);
        }
        self.client.send_json(req).await
    }

    /// Reports that a session has begun playing an item (legacy endpoint).
    ///
    /// OpenAPI: `POST /PlayingItems/{itemId}` (`OnPlaybackStart`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn on_playback_start(
        &self,
        item_id: uuid::Uuid,
        query: OnPlaybackQuery,
    ) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, &format!("PlayingItems/{item_id}"))?
            .query(query.params());
        self.client.send_unit(req).await
    }

    /// Reports playback progress (legacy endpoint).
    ///
    /// OpenAPI: `POST /PlayingItems/{itemId}/Progress` (`OnPlaybackProgress`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn on_playback_progress(
        &self,
        item_id: uuid::Uuid,
        query: OnPlaybackQuery,
    ) -> Result<()> {
        let req = self
            .client
            .request(Method::POST, &format!("PlayingItems/{item_id}/Progress"))?
            .query(query.params());
        self.client.send_unit(req).await
    }

    /// Reports playback stopped (legacy endpoint).
    ///
    /// OpenAPI: `DELETE /PlayingItems/{itemId}` (`OnPlaybackStopped`).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn on_playback_stopped(
        &self,
        item_id: uuid::Uuid,
        query: OnPlaybackQuery,
    ) -> Result<()> {
        let req = self
            .client
            .request(Method::DELETE, &format!("PlayingItems/{item_id}"))?
            .query(query.params());
        self.client.send_unit(req).await
    }
}
