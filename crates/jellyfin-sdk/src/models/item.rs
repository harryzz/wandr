use serde::{Deserialize, Serialize};

/// The base item kind.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum BaseItemKind {
    AggregateFolder,
    Audio,
    AudioBook,
    BasePluginFolder,
    Book,
    BoxSet,
    Channel,
    ChannelFolderItem,
    CollectionFolder,
    Episode,
    Folder,
    Genre,
    ManualPlaylistsFolder,
    Movie,
    LiveTvChannel,
    LiveTvProgram,
    MusicAlbum,
    MusicArtist,
    MusicGenre,
    MusicVideo,
    Person,
    Photo,
    PhotoAlbum,
    Playlist,
    PlaylistsFolder,
    Program,
    Recording,
    Season,
    Series,
    Studio,
    Trailer,
    TvChannel,
    TvProgram,
    UserRootFolder,
    UserView,
    Video,
    Year,
}

impl BaseItemKind {
    /// Returns the wire representation used by Jellyfin.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AggregateFolder => "AggregateFolder",
            Self::Audio => "Audio",
            Self::AudioBook => "AudioBook",
            Self::BasePluginFolder => "BasePluginFolder",
            Self::Book => "Book",
            Self::BoxSet => "BoxSet",
            Self::Channel => "Channel",
            Self::ChannelFolderItem => "ChannelFolderItem",
            Self::CollectionFolder => "CollectionFolder",
            Self::Episode => "Episode",
            Self::Folder => "Folder",
            Self::Genre => "Genre",
            Self::ManualPlaylistsFolder => "ManualPlaylistsFolder",
            Self::Movie => "Movie",
            Self::LiveTvChannel => "LiveTvChannel",
            Self::LiveTvProgram => "LiveTvProgram",
            Self::MusicAlbum => "MusicAlbum",
            Self::MusicArtist => "MusicArtist",
            Self::MusicGenre => "MusicGenre",
            Self::MusicVideo => "MusicVideo",
            Self::Person => "Person",
            Self::Photo => "Photo",
            Self::PhotoAlbum => "PhotoAlbum",
            Self::Playlist => "Playlist",
            Self::PlaylistsFolder => "PlaylistsFolder",
            Self::Program => "Program",
            Self::Recording => "Recording",
            Self::Season => "Season",
            Self::Series => "Series",
            Self::Studio => "Studio",
            Self::Trailer => "Trailer",
            Self::TvChannel => "TvChannel",
            Self::TvProgram => "TvProgram",
            Self::UserRootFolder => "UserRootFolder",
            Self::UserView => "UserView",
            Self::Video => "Video",
            Self::Year => "Year",
        }
    }
}

impl std::fmt::Display for BaseItemKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A minimal `BaseItemDto` subset for browsing and listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BaseItemStub {
    /// The item id.
    pub id: Option<uuid::Uuid>,
    /// The display name.
    pub name: Option<String>,
    /// The item kind.
    #[serde(rename = "Type")]
    pub kind: Option<BaseItemKind>,
    /// The parent item id.
    pub parent_id: Option<uuid::Uuid>,
    /// Whether this item is a folder.
    pub is_folder: Option<bool>,

    /// Production year.
    pub production_year: Option<i32>,
    /// Genres.
    pub genres: Option<Vec<String>>,
    /// Tags.
    pub tags: Option<Vec<String>>,
}
