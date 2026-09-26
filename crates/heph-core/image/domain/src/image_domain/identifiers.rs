use super::errors::ImageCatalogValueError;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use uuid::Uuid;

/// Stable identity for one cataloged OCI image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OciImageId(Uuid);

impl OciImageId {
    /// Creates a new image identity.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    /// Reconstructs an identity from its UUID representation.
    #[must_use]
    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }
    /// Returns the UUID representation.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for OciImageId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for OciImageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl FromStr for OciImageId {
    type Err = uuid::Error;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// Stable catalog key selected by execution contracts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ImageKey(String);

impl ImageKey {
    /// Parses a bounded lowercase key.
    ///
    /// # Errors
    ///
    /// Returns [`ImageCatalogValueError::InvalidKey`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, ImageCatalogValueError> {
        let value = value.into();
        let valid = (1..=64).contains(&value.len())
            && value.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || ((byte == b'_' || byte == b'-') && index > 0)
            });
        valid
            .then_some(Self(value))
            .ok_or(ImageCatalogValueError::InvalidKey)
    }
    /// Returns the validated key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ImageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Immutable OCI registry reference.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OciImageReference(String);

impl OciImageReference {
    /// Parses a digest-pinned OCI image reference.
    ///
    /// # Errors
    ///
    /// Returns [`ImageCatalogValueError::UnpinnedImage`] unless the reference
    /// ends in a lowercase SHA-256 digest.
    pub fn parse(value: impl Into<String>) -> Result<Self, ImageCatalogValueError> {
        let value = value.into();
        let Some((repository, digest)) = value.rsplit_once("@sha256:") else {
            return Err(ImageCatalogValueError::UnpinnedImage);
        };
        let valid_repository = !repository.is_empty()
            && repository == repository.trim()
            && !repository
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace());
        let valid_digest = digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
        (valid_repository && valid_digest)
            .then_some(Self(value))
            .ok_or(ImageCatalogValueError::UnpinnedImage)
    }
    /// Returns the complete immutable reference.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Returns the digest embedded in the reference.
    ///
    /// # Errors
    ///
    /// Returns [`ImageCatalogValueError::InvalidOciDigest`] for invalid
    /// deserialized state.
    pub fn digest(&self) -> Result<OciDigest, ImageCatalogValueError> {
        let (repository, digest) = self
            .0
            .rsplit_once('@')
            .ok_or(ImageCatalogValueError::InvalidOciDigest)?;
        if repository.is_empty()
            || repository
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        {
            return Err(ImageCatalogValueError::InvalidOciDigest);
        }
        OciDigest::parse(digest)
    }
}

impl fmt::Display for OciImageReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// An immutable OCI SHA-256 digest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OciDigest(String);

impl OciDigest {
    /// Parses a lowercase `sha256:` digest.
    ///
    /// # Errors
    ///
    /// Returns [`ImageCatalogValueError::InvalidOciDigest`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, ImageCatalogValueError> {
        let value = value.into();
        let valid = value.strip_prefix("sha256:").is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        });
        valid
            .then_some(Self(value))
            .ok_or(ImageCatalogValueError::InvalidOciDigest)
    }
    /// Returns the canonical digest string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OciDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl FromStr for OciDigest {
    type Err = ImageCatalogValueError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}
