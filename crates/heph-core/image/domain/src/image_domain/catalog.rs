use super::{
    errors::{ImageCatalogValueError, ImageSelectionError},
    identifiers::{ImageKey, OciImageId, OciImageReference},
};
use serde::{Deserialize, Serialize};

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

/// The boundary at which a cataloged OCI image may be selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageRole {
    /// An image that an authorized project may select for a build or guest.
    Execution,
    /// An administrator-owned image used only by a fixed platform operation.
    PlatformOperation,
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
    /// Whether the image is tenant-selectable or reserved for a platform task.
    pub role: ImageRole,
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
        if self.role == ImageRole::PlatformOperation {
            return Err(ImageSelectionError::PlatformOperationOnly);
        }
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
