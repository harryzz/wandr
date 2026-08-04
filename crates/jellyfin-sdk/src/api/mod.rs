//! API endpoint groups.

mod api_key;
mod configuration;
mod dynamic_hls;
mod filters;
mod genres;
mod images;
mod items;
mod library_structure;
mod movies;
mod playlists;
mod playstate;
mod plugins;
mod scheduled_tasks;
mod search;
mod sessions;
mod subtitles;
mod suggestions;
mod system;
mod user;
mod user_library;
mod user_views;
mod videos;

/// API key endpoints.
pub use api_key::ApiKeyApi;
/// Server configuration endpoints.
pub use configuration::ConfigurationApi;
/// Dynamic HLS endpoints.
pub use dynamic_hls::{DynamicHlsApi, HlsPlaylistQuery, HlsSegmentQuery};
/// Filter endpoints.
pub use filters::{FiltersApi, FiltersQuery};
/// Genres endpoints.
pub use genres::{GenresApi, GenresQuery};
/// Image endpoints (user profile image, etc).
pub use images::ImagesApi;
/// Options for `GET /UserImage`.
pub use images::UserImageRequest;
/// Options for `GET /Items/{itemId}/Images/{imageType}`.
pub use items::ItemImageRequest;
/// Items-related endpoints.
pub use items::ItemsApi;
/// Query parameters for `GET /Items`.
pub use items::ItemsQuery;
/// Query parameters for `POST /Items/{itemId}/Refresh`.
pub use items::RefreshItemQuery;
/// Query parameters for `GET /Items/{itemId}/Similar`.
pub use items::SimilarItemsQuery;
/// LibraryStructure endpoints.
pub use library_structure::LibraryStructureApi;
/// Movie endpoints.
pub use movies::{MovieRecommendationsQuery, MoviesApi};
/// Playlist endpoints.
pub use playlists::{PlaylistItemsQuery, PlaylistsApi};
/// Playstate endpoints.
pub use playstate::{OnPlaybackQuery, PlaystateApi};
/// Plugin endpoints.
pub use plugins::PluginsApi;
/// ScheduledTasks endpoints.
pub use scheduled_tasks::{ScheduledTasksApi, ScheduledTasksQuery};
/// Search endpoints.
pub use search::{SearchApi, SearchHintsQuery};
/// Options and query parameters for sessions and remote playback.
pub use sessions::{
    PlayOptions, PlaystateOptions, RemoteSession, SessionSelector, SessionsApi, SessionsQuery,
};
/// Subtitle endpoints.
pub use subtitles::SubtitlesApi;
/// Suggestions endpoints.
pub use suggestions::{SuggestionsApi, SuggestionsQuery};
/// System-related endpoints.
pub use system::SystemApi;
/// User/authentication related endpoints.
pub use user::UserApi;
/// User library browsing endpoints.
pub use user_library::{LatestMediaQuery, ResumeItemsQuery, UserLibraryApi};
/// User views (top-level categories) endpoints.
pub use user_views::{UserViewsApi, UserViewsQuery};
/// Videos endpoints.
pub use videos::VideosApi;
