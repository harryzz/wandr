//! Request/response models (DTOs).

mod api_key;
mod commands;
mod common;
mod filters;
mod image;
mod image_info;
mod item;
mod item_detail;
mod item_refresh;
mod items_query;
mod library_structure;
mod playback;
mod playlists;
mod playstate;
mod plugins;
mod query;
mod recommendations;
mod scheduled_tasks;
mod search;
mod session;
mod subtitles;
mod system;
mod user;
mod user_data;
mod user_stub;
mod user_views;
mod video_stream;

/// API key (access token) management DTOs.
pub use api_key::AuthenticationInfo;
/// Play/Playstate command enums.
pub use commands::{PlayCommand, PlaystateCommand};
/// Common name/id pairs.
pub use common::NameGuidPair;
/// Faceted browsing filters.
pub use filters::QueryFilters;
/// Image-related enums.
pub use image::{ImageFormat, ImageType};
/// Item image info DTO.
pub use image_info::ImageInfo;
/// A minimal `BaseItemDto` subset.
pub use item::{BaseItemKind, BaseItemStub};
/// A minimal `BaseItemDto` subset for item details.
pub use item_detail::BaseItemDetailStub;
/// Refresh-related enums.
pub use item_refresh::MetadataRefreshMode;
/// Common enums for item queries.
pub use items_query::{ItemField, ItemSortBy, SortOrder};
/// Library structure management DTOs.
pub use library_structure::{
    AddVirtualFolderBody, CollectionType, MediaPath, MediaPathInfo, UpdateLibraryOptionsRequest,
    UpdateMediaPathRequest, VirtualFolderInfo,
};
/// Playback and media info models.
pub use playback::{
    MediaSourceInfoStub, PlaybackErrorCode, PlaybackInfoRequest, PlaybackInfoResponseStub,
};
/// Playlist DTOs.
pub use playlists::{
    CreatePlaylist, PlaylistCreationResult, PlaylistDto, PlaylistUserPermissions, UpdatePlaylist,
    UpdatePlaylistUser,
};
/// Playstate reporting models.
pub use playstate::{PlayMethod, PlaybackProgress, PlaybackStart, PlaybackStop};
/// Plugin-related DTOs.
pub use plugins::{PluginInfo, PluginStatus};
/// Common query result container.
pub use query::QueryResult;
/// Recommendation DTO subset.
pub use recommendations::{RecommendationStub, RecommendationType};
/// Scheduled task DTO subset.
pub use scheduled_tasks::{
    DayOfWeek, TaskCompletionStatus, TaskInfo, TaskResult, TaskState, TaskTriggerInfo,
    TaskTriggerInfoType,
};
/// Search-related models.
pub use search::{MediaType, SearchHintResultStub, SearchHintStub};
/// Session-related models.
pub use session::{PlayerStateStub, SessionInfoStub};
/// Subtitle helper types.
pub use subtitles::{
    FontFile, RemoteSubtitleInfo, SubtitleFormat, SubtitleStreamQuery, UploadSubtitleRequest,
};
/// Public information about a Jellyfin server.
pub use system::PublicSystemInfo;
/// Request/response types for authentication by name.
pub use user::{AuthenticateUserByName, AuthenticationResult};
/// Per-user item data DTOs.
pub use user_data::{UpdateUserItemData, UserItemData};
/// A minimal user DTO subset.
pub use user_stub::UserStub;
/// UserViews related DTOs.
pub use user_views::SpecialViewOption;
/// Video streaming query helpers.
pub use video_stream::VideoStreamQuery;
