use serde::{Deserialize, Serialize};

use crate::models::BaseItemStub;

/// A minimal subset of `PlayerStateInfo`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlayerStateStub {
    /// Current playback position in ticks (100ns).
    pub position_ticks: Option<i64>,
    /// Whether the player is paused.
    pub is_paused: Option<bool>,
    /// Whether the player can seek.
    pub can_seek: Option<bool>,
    /// The current volume level.
    pub volume_level: Option<i32>,
    /// The currently selected media source id.
    pub media_source_id: Option<String>,
}

/// A minimal subset of `SessionInfoDto`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SessionInfoStub {
    /// The session id.
    pub id: Option<String>,
    /// The user id owning this session.
    pub user_id: Option<uuid::Uuid>,
    /// The username owning this session.
    pub user_name: Option<String>,
    /// The client name.
    pub client: Option<String>,
    /// The device name.
    pub device_name: Option<String>,
    /// The device type.
    pub device_type: Option<String>,
    /// The device id.
    pub device_id: Option<String>,
    /// Whether this session is active.
    pub is_active: Option<bool>,
    /// Whether this session supports media control.
    pub supports_media_control: Option<bool>,
    /// Whether this session supports remote control.
    pub supports_remote_control: Option<bool>,
    /// The current play state.
    pub play_state: Option<PlayerStateStub>,
    /// The currently playing item (subset).
    pub now_playing_item: Option<BaseItemStub>,
}
