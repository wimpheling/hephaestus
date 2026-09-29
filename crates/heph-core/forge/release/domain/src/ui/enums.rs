//! Enumerated release UI declaration values.

use crate::ReleaseValueError;
use serde::{Deserialize, Serialize};

/// Installation scope for a release UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiScope {
    /// Visible in one project.
    Project,
    /// Visible for one repository.
    Repository,
    /// Explicitly installed as a global interface.
    Global,
}

/// Generic repository Git authority explicitly declared by a release UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiRepositoryGitAccess {
    /// The UI receives no repository Git authority.
    None,
    /// The UI may read the explicitly bound repository.
    Read,
    /// The UI may read and write the explicitly bound repository.
    ReadWrite,
}

impl UiRepositoryGitAccess {
    /// Returns the stable database and manifest spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Read => "read",
            Self::ReadWrite => "read_write",
        }
    }

    /// Parses the stable database and manifest spelling.
    ///
    /// # Errors
    ///
    /// Returns [`ReleaseValueError::InvalidUiRepositoryGitAccess`] for an
    /// unknown value.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, ReleaseValueError> {
        match value.as_ref() {
            "none" => Ok(Self::None),
            "read" => Ok(Self::Read),
            "read_write" => Ok(Self::ReadWrite),
            _ => Err(ReleaseValueError::InvalidUiRepositoryGitAccess),
        }
    }
}

impl Default for UiRepositoryGitAccess {
    fn default() -> Self {
        Self::None
    }
}

/// Initial host presentation for a release UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiPresentation {
    /// Render within a bounded iframe.
    Iframe,
    /// Open as a bounded full-page route.
    FullPage,
}

/// Published cache behavior for release-owned UI content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiCachePolicy {
    /// Revalidate every request and forbid browser/intermediary storage.
    NoStore,
}

/// Allowlisted icon family for a release UI tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiIcon {
    /// General application icon.
    App,
    /// Chat or assistant icon.
    Chat,
    /// Code or developer icon.
    Code,
    /// Documentation or book icon.
    Book,
    /// Metrics or chart icon.
    Chart,
}

impl UiIcon {
    /// Returns the stable lowercase wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Chat => "chat",
            Self::Code => "code",
            Self::Book => "book",
            Self::Chart => "chart",
        }
    }
}

/// Explicitly allowed UI artifact media type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum UiMediaType {
    /// HTML document.
    TextHtml,
    /// CSS stylesheet.
    TextCss,
    /// JavaScript source.
    TextJavascript,
    /// JSON data.
    ApplicationJson,
    /// Plain text.
    TextPlain,
    /// PNG image.
    ImagePng,
    /// JPEG image.
    ImageJpeg,
    /// WebP image.
    ImageWebp,
    /// GIF image.
    ImageGif,
    /// SVG image.
    ImageSvgXml,
    /// WebAssembly module.
    ApplicationWasm,
}

impl UiMediaType {
    /// Returns the stable explicit MIME spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TextHtml => "text/html",
            Self::TextCss => "text/css",
            Self::TextJavascript => "text/javascript",
            Self::ApplicationJson => "application/json",
            Self::TextPlain => "text/plain",
            Self::ImagePng => "image/png",
            Self::ImageJpeg => "image/jpeg",
            Self::ImageWebp => "image/webp",
            Self::ImageGif => "image/gif",
            Self::ImageSvgXml => "image/svg+xml",
            Self::ApplicationWasm => "application/wasm",
        }
    }
}

impl TryFrom<String> for UiMediaType {
    type Error = ReleaseValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "text/html" => Ok(Self::TextHtml),
            "text/css" => Ok(Self::TextCss),
            "text/javascript" => Ok(Self::TextJavascript),
            "application/json" => Ok(Self::ApplicationJson),
            "text/plain" => Ok(Self::TextPlain),
            "image/png" => Ok(Self::ImagePng),
            "image/jpeg" => Ok(Self::ImageJpeg),
            "image/webp" => Ok(Self::ImageWebp),
            "image/gif" => Ok(Self::ImageGif),
            "image/svg+xml" => Ok(Self::ImageSvgXml),
            "application/wasm" => Ok(Self::ApplicationWasm),
            _ => Err(ReleaseValueError::UnsupportedUiMediaType),
        }
    }
}

impl From<UiMediaType> for String {
    fn from(value: UiMediaType) -> Self {
        value.as_str().to_owned()
    }
}
