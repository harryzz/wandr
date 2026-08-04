use serde::{Deserialize, Serialize};

/// Item image type.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ImageType {
    Primary,
    Art,
    Backdrop,
    Banner,
    Logo,
    Thumb,
    Disc,
    Box,
    Screenshot,
    Menu,
    Chapter,
    BoxRear,
    Profile,
}

impl ImageType {
    /// Returns the wire representation used by Jellyfin.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "Primary",
            Self::Art => "Art",
            Self::Backdrop => "Backdrop",
            Self::Banner => "Banner",
            Self::Logo => "Logo",
            Self::Thumb => "Thumb",
            Self::Disc => "Disc",
            Self::Box => "Box",
            Self::Screenshot => "Screenshot",
            Self::Menu => "Menu",
            Self::Chapter => "Chapter",
            Self::BoxRear => "BoxRear",
            Self::Profile => "Profile",
        }
    }
}

impl std::fmt::Display for ImageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Image output format.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ImageFormat {
    Bmp,
    Gif,
    Jpg,
    Png,
    Webp,
    Svg,
}

impl ImageFormat {
    /// Returns the wire representation used by Jellyfin.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bmp => "Bmp",
            Self::Gif => "Gif",
            Self::Jpg => "Jpg",
            Self::Png => "Png",
            Self::Webp => "Webp",
            Self::Svg => "Svg",
        }
    }
}

impl std::fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
