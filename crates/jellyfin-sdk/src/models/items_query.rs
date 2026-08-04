use serde::{Deserialize, Serialize};

/// Additional item fields to request in list/detail responses.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ItemField {
    AirTime,
    CanDelete,
    CanDownload,
    ChannelInfo,
    Chapters,
    Trickplay,
    ChildCount,
    CumulativeRunTimeTicks,
    CustomRating,
    DateCreated,
    DateLastMediaAdded,
    DisplayPreferencesId,
    Etag,
    ExternalUrls,
    Genres,
    ItemCounts,
    MediaSourceCount,
    MediaSources,
    OriginalTitle,
    Overview,
    ParentId,
    Path,
    People,
    PlayAccess,
    ProductionLocations,
    ProviderIds,
    PrimaryImageAspectRatio,
    RecursiveItemCount,
    Settings,
    SeriesStudio,
    SortName,
    SpecialEpisodeNumbers,
    Studios,
    Taglines,
    Tags,
    RemoteTrailers,
    MediaStreams,
    SeasonUserData,
    DateLastRefreshed,
    DateLastSaved,
    RefreshState,
    ChannelImage,
    EnableMediaSourceDisplay,
    Width,
    Height,
    ExtraIds,
    LocalTrailerCount,
    IsHD,
    SpecialFeatureCount,
}

impl ItemField {
    /// Returns the wire representation used by Jellyfin.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AirTime => "AirTime",
            Self::CanDelete => "CanDelete",
            Self::CanDownload => "CanDownload",
            Self::ChannelInfo => "ChannelInfo",
            Self::Chapters => "Chapters",
            Self::Trickplay => "Trickplay",
            Self::ChildCount => "ChildCount",
            Self::CumulativeRunTimeTicks => "CumulativeRunTimeTicks",
            Self::CustomRating => "CustomRating",
            Self::DateCreated => "DateCreated",
            Self::DateLastMediaAdded => "DateLastMediaAdded",
            Self::DisplayPreferencesId => "DisplayPreferencesId",
            Self::Etag => "Etag",
            Self::ExternalUrls => "ExternalUrls",
            Self::Genres => "Genres",
            Self::ItemCounts => "ItemCounts",
            Self::MediaSourceCount => "MediaSourceCount",
            Self::MediaSources => "MediaSources",
            Self::OriginalTitle => "OriginalTitle",
            Self::Overview => "Overview",
            Self::ParentId => "ParentId",
            Self::Path => "Path",
            Self::People => "People",
            Self::PlayAccess => "PlayAccess",
            Self::ProductionLocations => "ProductionLocations",
            Self::ProviderIds => "ProviderIds",
            Self::PrimaryImageAspectRatio => "PrimaryImageAspectRatio",
            Self::RecursiveItemCount => "RecursiveItemCount",
            Self::Settings => "Settings",
            Self::SeriesStudio => "SeriesStudio",
            Self::SortName => "SortName",
            Self::SpecialEpisodeNumbers => "SpecialEpisodeNumbers",
            Self::Studios => "Studios",
            Self::Taglines => "Taglines",
            Self::Tags => "Tags",
            Self::RemoteTrailers => "RemoteTrailers",
            Self::MediaStreams => "MediaStreams",
            Self::SeasonUserData => "SeasonUserData",
            Self::DateLastRefreshed => "DateLastRefreshed",
            Self::DateLastSaved => "DateLastSaved",
            Self::RefreshState => "RefreshState",
            Self::ChannelImage => "ChannelImage",
            Self::EnableMediaSourceDisplay => "EnableMediaSourceDisplay",
            Self::Width => "Width",
            Self::Height => "Height",
            Self::ExtraIds => "ExtraIds",
            Self::LocalTrailerCount => "LocalTrailerCount",
            Self::IsHD => "IsHD",
            Self::SpecialFeatureCount => "SpecialFeatureCount",
        }
    }
}

impl std::fmt::Display for ItemField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Sort fields for item queries.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ItemSortBy {
    Default,
    AiredEpisodeOrder,
    Album,
    AlbumArtist,
    Artist,
    DateCreated,
    OfficialRating,
    DatePlayed,
    PremiereDate,
    StartDate,
    SortName,
    Name,
    Random,
    Runtime,
    CommunityRating,
    ProductionYear,
    PlayCount,
    CriticRating,
    IsFolder,
    IsUnplayed,
    IsPlayed,
    SeriesSortName,
    VideoBitRate,
    AirTime,
    Studio,
    IsFavoriteOrLiked,
    DateLastContentAdded,
    SeriesDatePlayed,
    ParentIndexNumber,
    IndexNumber,
}

impl ItemSortBy {
    /// Returns the wire representation used by Jellyfin.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::AiredEpisodeOrder => "AiredEpisodeOrder",
            Self::Album => "Album",
            Self::AlbumArtist => "AlbumArtist",
            Self::Artist => "Artist",
            Self::DateCreated => "DateCreated",
            Self::OfficialRating => "OfficialRating",
            Self::DatePlayed => "DatePlayed",
            Self::PremiereDate => "PremiereDate",
            Self::StartDate => "StartDate",
            Self::SortName => "SortName",
            Self::Name => "Name",
            Self::Random => "Random",
            Self::Runtime => "Runtime",
            Self::CommunityRating => "CommunityRating",
            Self::ProductionYear => "ProductionYear",
            Self::PlayCount => "PlayCount",
            Self::CriticRating => "CriticRating",
            Self::IsFolder => "IsFolder",
            Self::IsUnplayed => "IsUnplayed",
            Self::IsPlayed => "IsPlayed",
            Self::SeriesSortName => "SeriesSortName",
            Self::VideoBitRate => "VideoBitRate",
            Self::AirTime => "AirTime",
            Self::Studio => "Studio",
            Self::IsFavoriteOrLiked => "IsFavoriteOrLiked",
            Self::DateLastContentAdded => "DateLastContentAdded",
            Self::SeriesDatePlayed => "SeriesDatePlayed",
            Self::ParentIndexNumber => "ParentIndexNumber",
            Self::IndexNumber => "IndexNumber",
        }
    }
}

impl std::fmt::Display for ItemSortBy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Sort order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum SortOrder {
    /// Ascending.
    Ascending,
    /// Descending.
    Descending,
}

impl SortOrder {
    /// Returns the wire representation used by Jellyfin.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ascending => "Ascending",
            Self::Descending => "Descending",
        }
    }
}

impl std::fmt::Display for SortOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
