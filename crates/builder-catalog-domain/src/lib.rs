//! Provider-neutral contracts for immutable OCI images.

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

/// Availability of an OCI image for new execution contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityState {
    /// The image can be selected for new execution contracts.
    Available,
    /// The image is retained but temporarily cannot be selected.
    Unavailable,
    /// The image is historical-only and cannot be selected.
    Retired,
}

/// One pinned toolchain advertised by an image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Toolchain {
    /// Stable toolchain name.
    pub name: String,
    /// Exact advertised version.
    pub version: String,
}

/// Supply-chain provenance retained with an OCI image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageProvenance {
    /// Source or build attestation URI.
    pub source: String,
    /// Optional signature evidence reference.
    pub signature: Option<String>,
    /// Optional SBOM evidence reference.
    pub sbom: Option<String>,
}

/// One cataloged immutable OCI image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OciImage {
    /// Stable catalog identity.
    pub id: OciImageId,
    /// Stable key.
    pub key: ImageKey,
    /// Human-readable name.
    pub display_name: String,
    /// Immutable image reference.
    pub image_reference: OciImageReference,
    /// Toolchain metadata.
    pub toolchains: Vec<Toolchain>,
    /// Architectures declared by the immutable OCI manifest.
    pub architectures: Vec<String>,
    /// Availability for new work.
    pub availability: AvailabilityState,
    /// Supply-chain provenance.
    pub provenance: ImageProvenance,
    /// Platform policy version that approved the catalog record.
    pub platform_policy_version: String,
}

impl OciImage {
    /// Validates immutable catalog metadata.
    ///
    /// # Errors
    ///
    /// Returns [`ImageCatalogValueError`] for invalid metadata.
    pub fn validate(&self) -> Result<(), ImageCatalogValueError> {
        if self.id.as_uuid().is_nil()
            || self.display_name.trim().is_empty()
            || self.display_name.len() > 200
        {
            return Err(ImageCatalogValueError::InvalidDisplayName);
        }
        ImageKey::parse(self.key.as_str().to_owned())?;
        OciImageReference::parse(self.image_reference.as_str().to_owned())?;
        if self.toolchains.iter().any(|toolchain| {
            toolchain.name.trim().is_empty()
                || toolchain.name.len() > 64
                || toolchain.version.trim().is_empty()
                || toolchain.version.len() > 128
        }) {
            return Err(ImageCatalogValueError::InvalidToolchain);
        }
        if self.architectures.is_empty()
            || self.architectures.len() > 32
            || self.architectures.iter().any(|architecture| {
                architecture.is_empty()
                    || architecture.len() > 64
                    || architecture
                        .bytes()
                        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
            })
        {
            return Err(ImageCatalogValueError::InvalidArchitecture);
        }
        if self.provenance.source.trim().is_empty()
            || self.provenance.source.len() > 2048
            || self.platform_policy_version.trim().is_empty()
            || self.platform_policy_version.len() > 128
        {
            return Err(ImageCatalogValueError::MissingProvenance);
        }
        Ok(())
    }
    /// Resolves this catalog row to immutable execution provenance.
    ///
    /// # Errors
    ///
    /// Returns an error unless the image is available for new work.
    pub fn resolve(&self) -> Result<ResolvedImage, ImageSelectionError> {
        match self.availability {
            AvailabilityState::Available => Ok(ResolvedImage {
                image_id: self.id,
                key: self.key.clone(),
                image_reference: self.image_reference.clone(),
                platform_policy_version: self.platform_policy_version.clone(),
            }),
            AvailabilityState::Unavailable => Err(ImageSelectionError::Unavailable),
            AvailabilityState::Retired => Err(ImageSelectionError::Retired),
        }
    }
}

/// Immutable OCI-image provenance stored with a build or release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedImage {
    /// Stable catalog identity at resolution time.
    pub image_id: OciImageId,
    /// Human-selected stable key.
    pub key: ImageKey,
    /// Exact OCI reference frozen into provenance.
    pub image_reference: OciImageReference,
    /// Policy version used at resolution time.
    pub platform_policy_version: String,
}

/// Durable OCI registry publication state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryPublicationState {
    /// A publisher has not started the immutable publication.
    Pending,
    /// A trusted publisher owns the current attempt.
    Publishing,
    /// Required evidence has been verified but not approved.
    Verified,
    /// The immutable digest is approved for consumption.
    Approved,
    /// Previously approved content is absent or inconsistent.
    Missing,
    /// The publication is retained only for historical inspection.
    Retired,
}

/// Consumer-visible availability projected from publication state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryAvailabilityState {
    /// The publication is approved and present.
    Available,
    /// The publication is not usable for new work.
    Unavailable,
    /// The publication is historical-only.
    Retired,
}

/// Verification state for one supply-chain evidence kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryEvidenceState {
    /// Verification has not produced evidence yet.
    Pending,
    /// Evidence was verified for the subject digest.
    Verified,
    /// The bound policy does not require this evidence kind.
    NotRequired,
}

/// One immutable OCI evidence reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEvidence {
    /// Durable verification status.
    pub state: RegistryEvidenceState,
    /// Verified immutable evidence reference, when present.
    pub immutable_reference: Option<OciImageReference>,
}

impl RegistryEvidence {
    /// Returns evidence awaiting verification.
    #[must_use]
    pub const fn pending() -> Self {
        Self {
            state: RegistryEvidenceState::Pending,
            immutable_reference: None,
        }
    }
    /// Returns verified evidence.
    #[must_use]
    pub const fn verified(reference: OciImageReference) -> Self {
        Self {
            state: RegistryEvidenceState::Verified,
            immutable_reference: Some(reference),
        }
    }
    /// Returns a policy-excluded evidence kind.
    #[must_use]
    pub const fn not_required() -> Self {
        Self {
            state: RegistryEvidenceState::NotRequired,
            immutable_reference: None,
        }
    }
}

/// Safe projection of a registry publication and its supply-chain evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryPublication {
    /// Durable control-plane state.
    pub state: RegistryPublicationState,
    /// Consumer-visible state derived from the durable state.
    pub availability: RegistryAvailabilityState,
    /// Approved or expected immutable manifest reference.
    pub immutable_reference: Option<OciImageReference>,
    /// Architectures verified from the immutable manifest.
    pub architectures: Vec<String>,
    /// SBOM evidence.
    pub sbom: RegistryEvidence,
    /// Build provenance evidence.
    pub provenance: RegistryEvidence,
    /// Vulnerability scan evidence.
    pub scan: RegistryEvidence,
    /// Optional signature evidence.
    pub signature: RegistryEvidence,
}

impl RegistryPublication {
    /// Validates the safe projection at an adapter boundary.
    ///
    /// # Errors
    ///
    /// Returns [`ImageCatalogValueError::InvalidRegistryPublication`] for an
    /// inconsistent projection.
    pub fn validate(&self) -> Result<(), ImageCatalogValueError> {
        let valid_architectures = !self.architectures.is_empty()
            && self.architectures.len() <= 32
            && self.architectures.iter().all(|architecture| {
                !architecture.is_empty()
                    && architecture.len() <= 64
                    && !architecture
                        .bytes()
                        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
            });
        if !valid_architectures {
            return Err(ImageCatalogValueError::InvalidRegistryPublication);
        }
        if [&self.sbom, &self.provenance, &self.scan, &self.signature]
            .into_iter()
            .any(|evidence| {
                matches!(evidence.state, RegistryEvidenceState::Verified)
                    != evidence.immutable_reference.is_some()
            })
        {
            return Err(ImageCatalogValueError::InvalidRegistryPublication);
        }
        match self.state {
            RegistryPublicationState::Approved
                if self.availability == RegistryAvailabilityState::Available
                    && self.immutable_reference.is_some() =>
            {
                Ok(())
            }
            RegistryPublicationState::Retired
                if self.availability == RegistryAvailabilityState::Retired =>
            {
                Ok(())
            }
            RegistryPublicationState::Pending
            | RegistryPublicationState::Publishing
            | RegistryPublicationState::Verified
            | RegistryPublicationState::Missing
                if self.availability == RegistryAvailabilityState::Unavailable =>
            {
                Ok(())
            }
            _ => Err(ImageCatalogValueError::InvalidRegistryPublication),
        }
    }
}

/// Catalog metadata paired with safe registry evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciImagePublication {
    /// Catalog metadata.
    pub image: OciImage,
    /// Registry lifecycle and supply-chain evidence.
    pub registry_publication: RegistryPublication,
}

impl OciImagePublication {
    /// Validates both catalog and registry metadata.
    ///
    /// # Errors
    ///
    /// Returns invalid-data errors for either layer.
    pub fn validate(&self) -> Result<(), ImageCatalogValueError> {
        self.image.validate()?;
        self.registry_publication.validate()
    }
}

/// Catalog value validation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImageCatalogValueError {
    /// The stable key is malformed.
    #[error("image key must be a bounded lowercase identifier")]
    InvalidKey,
    /// The reference is not digest-pinned.
    #[error("OCI image reference must be digest-pinned with a lowercase SHA-256 digest")]
    UnpinnedImage,
    /// The digest is malformed.
    #[error("OCI digest must be a lowercase sha256 digest")]
    InvalidOciDigest,
    /// The display name is malformed.
    #[error("image display name is invalid")]
    InvalidDisplayName,
    /// Toolchain metadata is malformed.
    #[error("image toolchain metadata is invalid")]
    InvalidToolchain,
    /// Architecture metadata is malformed.
    #[error("image architecture metadata is invalid")]
    InvalidArchitecture,
    /// Required provenance or policy metadata is absent.
    #[error("image provenance and platform policy version are required")]
    MissingProvenance,
    /// Stored lifecycle data is unrecognized.
    #[error("image catalog contains an unknown lifecycle value")]
    InvalidStoredValue,
    /// Registry evidence is inconsistent.
    #[error("registry publication metadata is invalid")]
    InvalidRegistryPublication,
}

/// Failure when resolving an OCI image for an execution contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImageSelectionError {
    /// The image is temporarily unavailable.
    #[error("selected OCI image is unavailable")]
    Unavailable,
    /// The image is historical-only.
    #[error("selected OCI image was retired")]
    Retired,
}
