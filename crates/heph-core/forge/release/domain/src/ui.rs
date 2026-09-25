//! Validated primitives for release-owned distribution UI declarations.

use crate::ReleaseValueError;
use serde::{Deserialize, Serialize};
use std::fmt;

/// The only UI declaration schema version currently supported.
pub const SCHEMA_VERSION: u16 = 1;

/// Validates a UI declaration schema version without accepting future shapes.
///
/// # Errors
///
/// Returns [`ReleaseValueError::UnsupportedUiSchemaVersion`] for every version
/// other than [`SCHEMA_VERSION`].
pub const fn validate_schema_version(version: u16) -> Result<(), ReleaseValueError> {
    if version == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(ReleaseValueError::UnsupportedUiSchemaVersion)
    }
}

/// A stable lowercase UI key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct UiKey(String);

impl UiKey {
    /// Parses a key beginning with a lowercase ASCII letter and followed by
    /// lowercase ASCII letters, digits, or hyphens.
    ///
    /// # Errors
    ///
    /// Returns [`ReleaseValueError::InvalidUiKey`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
        let value = value.into();
        let mut bytes = value.bytes();
        let valid = (1..=64).contains(&value.len())
            && bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if valid {
            Ok(Self(value))
        } else {
            Err(ReleaseValueError::InvalidUiKey)
        }
    }

    /// Returns the validated key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for UiKey {
    type Error = ReleaseValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<UiKey> for String {
    fn from(value: UiKey) -> Self {
        value.0
    }
}

/// A platform-relative UI route path made of ASCII unreserved segments.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct UiRoutePath(String);

impl UiRoutePath {
    /// Parses a relative path without URL syntax or traversal segments.
    ///
    /// # Errors
    ///
    /// Returns [`ReleaseValueError::InvalidUiRoutePath`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
        let value = value.into();
        let valid = (1..=256).contains(&value.len())
            && value.bytes().all(is_ascii_unreserved_or_slash)
            && !value.starts_with('/')
            && !value.ends_with('/')
            && value
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
        if valid {
            Ok(Self(value))
        } else {
            Err(ReleaseValueError::InvalidUiRoutePath)
        }
    }

    /// Returns the validated relative path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const fn is_ascii_unreserved_or_slash(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
}

impl fmt::Display for UiRoutePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for UiRoutePath {
    type Error = ReleaseValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<UiRoutePath> for String {
    fn from(value: UiRoutePath) -> Self {
        value.0
    }
}

/// A bounded user-facing UI label.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct UiLabel(String);

impl UiLabel {
    /// Trims and validates a nonempty label of at most 80 Unicode characters.
    ///
    /// # Errors
    ///
    /// Returns [`ReleaseValueError::InvalidUiLabel`] for empty, oversized, or
    /// control-containing input.
    pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
        let value = value.into();
        let contains_forbidden_character = value.chars().any(|character| {
            character.is_control()
                || is_bidi_control(character)
                || matches!(character, '\u{2028}' | '\u{2029}')
        });
        let trimmed = value.trim();
        let valid =
            !contains_forbidden_character && !trimmed.is_empty() && trimmed.chars().count() <= 80;
        if valid {
            Ok(Self(trimmed.to_owned()))
        } else {
            Err(ReleaseValueError::InvalidUiLabel)
        }
    }

    /// Returns the trimmed validated label.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

impl fmt::Display for UiLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for UiLabel {
    type Error = ReleaseValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<UiLabel> for String {
    fn from(value: UiLabel) -> Self {
        value.0
    }
}

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

#[cfg(test)]
#[path = "ui/tests.rs"]
mod tests;
