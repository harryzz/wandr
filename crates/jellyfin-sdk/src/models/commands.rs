use serde::{Deserialize, Serialize};

/// A play command for `POST /Sessions/{sessionId}/Playing`.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PlayCommand {
    PlayNow,
    PlayNext,
    PlayLast,
    PlayInstantMix,
    PlayShuffle,
}

impl PlayCommand {
    /// Returns the wire representation used by Jellyfin.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PlayNow => "PlayNow",
            Self::PlayNext => "PlayNext",
            Self::PlayLast => "PlayLast",
            Self::PlayInstantMix => "PlayInstantMix",
            Self::PlayShuffle => "PlayShuffle",
        }
    }
}

impl std::fmt::Display for PlayCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A playstate command for `POST /Sessions/{sessionId}/Playing/{command}`.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PlaystateCommand {
    Stop,
    Pause,
    Unpause,
    NextTrack,
    PreviousTrack,
    Seek,
    Rewind,
    FastForward,
    PlayPause,
}

impl PlaystateCommand {
    /// Returns the wire representation used by Jellyfin.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "Stop",
            Self::Pause => "Pause",
            Self::Unpause => "Unpause",
            Self::NextTrack => "NextTrack",
            Self::PreviousTrack => "PreviousTrack",
            Self::Seek => "Seek",
            Self::Rewind => "Rewind",
            Self::FastForward => "FastForward",
            Self::PlayPause => "PlayPause",
        }
    }
}

impl std::fmt::Display for PlaystateCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
