//! Provider-neutral contracts for the platform-owned builder image catalog.

use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use uuid::Uuid;

/// Stable identity for one cataloged builder image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BuilderImageId(Uuid);

impl BuilderImageId {
    /// Creates a new catalog identity.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Creates an identity from its UUID representation.
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

impl Default for BuilderImageId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for BuilderImageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for BuilderImageId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// Stable catalog key referenced by platform tooling and UI.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BuilderKey(String);

impl BuilderKey {
    /// Parses a bounded lowercase key.
    ///
    /// # Errors
    ///
    /// Returns [`BuilderCatalogValueError::InvalidKey`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, BuilderCatalogValueError> {
        let value = value.into();
        let valid = (1..=64).contains(&value.len())
            && value.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || ((byte == b'_' || byte == b'-') && index > 0)
            });
        valid
            .then_some(Self(value))
            .ok_or(BuilderCatalogValueError::InvalidKey)
    }

    /// Returns the validated key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BuilderKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Immutable registry reference for a builder image.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BuilderImageReference(String);

impl BuilderImageReference {
    /// Parses an image reference with a lowercase SHA-256 digest.
    ///
    /// # Errors
    ///
    /// Returns [`BuilderCatalogValueError::UnpinnedImage`] when the reference
    /// is not immutable and digest-pinned.
    pub fn parse(value: impl Into<String>) -> Result<Self, BuilderCatalogValueError> {
        let value = value.into();
        let Some((repository, digest)) = value.rsplit_once("@sha256:") else {
            return Err(BuilderCatalogValueError::UnpinnedImage);
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
        if valid_repository && valid_digest {
            Ok(Self(value))
        } else {
            Err(BuilderCatalogValueError::UnpinnedImage)
        }
    }

    /// Returns the complete immutable reference.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BuilderImageReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Ordered build network ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildNetworkPolicy {
    /// No guest networking.
    Disabled,
    /// Broker connectivity only.
    BrokerOnly,
    /// Constrained external egress.
    Egress,
}

impl BuildNetworkPolicy {
    const fn rank(self) -> u8 {
        match self {
            Self::Disabled => 0,
            Self::BrokerOnly => 1,
            Self::Egress => 2,
        }
    }

    /// Returns whether the catalog ceiling permits the requested profile.
    #[must_use]
    pub const fn permits(self, requested: Self) -> bool {
        requested.rank() <= self.rank()
    }
}

/// Catalog preparation lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparationState {
    /// Image has passed platform preparation and can be selected.
    Ready,
    /// Image preparation is still in progress.
    Preparing,
    /// Preparation failed and the image cannot be selected.
    Failed,
}

/// Catalog availability lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityState {
    /// Image may be selected for new builds.
    Available,
    /// Image is temporarily unavailable.
    Unavailable,
    /// Image is retained for historical display but cannot be selected.
    Retired,
}

/// Dependency acquisition contract owned by the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyPolicy {
    /// All dependencies must be supplied by the source or image.
    VendoredOffline,
    /// Dependencies may use a read-only platform cache.
    ReadOnlyPlatformCache,
    /// Dependencies may use explicitly constrained package registries.
    ConstrainedRegistryEgress,
}

/// One pinned toolchain exposed by a builder image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Toolchain {
    /// Stable toolchain name, such as `rust` or `node`.
    pub name: String,
    /// Exact human-readable version selected by the image.
    pub version: String,
}

/// Platform provenance for a catalog entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuilderProvenance {
    /// Source or build attestation reference.
    pub source: String,
    /// Optional signature or attestation reference.
    pub signature: Option<String>,
    /// Optional SBOM reference.
    pub sbom: Option<String>,
}

/// One platform-owned, digest-pinned builder image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuilderImage {
    /// Stable catalog identity.
    pub id: BuilderImageId,
    /// Stable platform key.
    pub key: BuilderKey,
    /// Human-readable image name.
    pub display_name: String,
    /// Immutable image reference.
    pub image_reference: BuilderImageReference,
    /// Pinned tools available in the image.
    pub toolchains: Vec<Toolchain>,
    /// Supported guest architectures.
    pub architectures: Vec<String>,
    /// Preparation lifecycle.
    pub preparation: PreparationState,
    /// Availability lifecycle.
    pub availability: AvailabilityState,
    /// Maximum build network profile.
    pub network_ceiling: BuildNetworkPolicy,
    /// Maximum virtual CPUs accepted for this image.
    pub max_vcpus: u8,
    /// Maximum memory accepted for this image.
    pub max_memory_mib: u32,
    /// Dependency acquisition policy.
    pub dependency_policy: DependencyPolicy,
    /// Supply-chain provenance references.
    pub provenance: BuilderProvenance,
    /// Platform policy version used to approve this entry.
    pub platform_policy_version: String,
}

impl BuilderImage {
    /// Validates catalog invariants that must hold before persistence or use.
    ///
    /// # Errors
    ///
    /// Returns a stable value error for malformed metadata.
    pub fn validate(&self) -> Result<(), BuilderCatalogValueError> {
        if self.display_name.trim().is_empty() || self.display_name.len() > 200 {
            return Err(BuilderCatalogValueError::InvalidDisplayName);
        }
        if self.toolchains.is_empty()
            || self.toolchains.iter().any(|tool| {
                tool.name.trim().is_empty()
                    || tool.name.len() > 64
                    || tool.version.trim().is_empty()
                    || tool.version.len() > 128
            })
        {
            return Err(BuilderCatalogValueError::InvalidToolchain);
        }
        if self.architectures.is_empty()
            || self.architectures.iter().any(|architecture| {
                architecture.trim().is_empty()
                    || architecture.len() > 32
                    || architecture
                        .bytes()
                        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
            })
        {
            return Err(BuilderCatalogValueError::InvalidArchitecture);
        }
        if self.max_vcpus == 0 || self.max_memory_mib == 0 {
            return Err(BuilderCatalogValueError::InvalidResourceCeiling);
        }
        if self.provenance.source.trim().is_empty()
            || self.platform_policy_version.trim().is_empty()
        {
            return Err(BuilderCatalogValueError::MissingProvenance);
        }
        Ok(())
    }

    /// Validates one parsed `agent.toml` build selection against this image.
    ///
    /// # Errors
    ///
    /// Returns a selection error when the image is not runnable or the source
    /// configuration asks for a broader resource or network policy.
    pub fn validate_selection(
        &self,
        requested_network: BuildNetworkPolicy,
        requested_vcpus: u8,
        requested_memory_mib: u32,
    ) -> Result<ValidatedBuilderSelection, BuilderSelectionError> {
        match self.preparation {
            PreparationState::Ready => {}
            PreparationState::Preparing | PreparationState::Failed => {
                return Err(BuilderSelectionError::NotPrepared);
            }
        }
        match self.availability {
            AvailabilityState::Available => {}
            AvailabilityState::Unavailable => return Err(BuilderSelectionError::Unavailable),
            AvailabilityState::Retired => return Err(BuilderSelectionError::Retired),
        }
        if requested_vcpus == 0
            || requested_vcpus > self.max_vcpus
            || requested_memory_mib == 0
            || requested_memory_mib > self.max_memory_mib
        {
            return Err(BuilderSelectionError::ResourceCeilingExceeded);
        }
        if !self.network_ceiling.permits(requested_network) {
            return Err(BuilderSelectionError::NetworkCeilingExceeded);
        }
        Ok(ValidatedBuilderSelection {
            image_id: self.id,
            key: self.key.clone(),
            image_reference: self.image_reference.clone(),
            network: requested_network,
            vcpus: requested_vcpus,
            memory_mib: requested_memory_mib,
            platform_policy_version: self.platform_policy_version.clone(),
        })
    }
}

/// Immutable selection handed to build execution after validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedBuilderSelection {
    /// Stable image identity.
    pub image_id: BuilderImageId,
    /// Stable image key.
    pub key: BuilderKey,
    /// Exact image reference.
    pub image_reference: BuilderImageReference,
    /// Validated effective network profile.
    pub network: BuildNetworkPolicy,
    /// Validated vCPU selection.
    pub vcpus: u8,
    /// Validated memory selection.
    pub memory_mib: u32,
    /// Policy version under which selection was approved.
    pub platform_policy_version: String,
}

/// Catalog value validation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuilderCatalogValueError {
    /// Stable key is malformed.
    #[error("builder key must be a bounded lowercase identifier")]
    InvalidKey,
    /// Image reference lacks a lowercase SHA-256 digest.
    #[error("builder image reference must be digest-pinned with a lowercase SHA-256 digest")]
    UnpinnedImage,
    /// Display name is outside its bounds.
    #[error("builder display name is invalid")]
    InvalidDisplayName,
    /// Toolchain metadata is incomplete.
    #[error("builder toolchain metadata is invalid")]
    InvalidToolchain,
    /// Architecture metadata is incomplete.
    #[error("builder architecture metadata is invalid")]
    InvalidArchitecture,
    /// Resource ceiling is not positive.
    #[error("builder resource ceiling must be positive")]
    InvalidResourceCeiling,
    /// Provenance or platform policy metadata is missing.
    #[error("builder provenance and platform policy version are required")]
    MissingProvenance,
    /// Stored lifecycle or policy text is not recognized.
    #[error("builder catalog contains an unknown lifecycle or policy value")]
    InvalidStoredValue,
}

/// Failure when selecting an image for one source build.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuilderSelectionError {
    /// No catalog entry matches the exact source reference.
    #[error("agent.toml selects an unknown builder image")]
    UnknownImage,
    /// Image is not prepared.
    #[error("selected builder image is not prepared")]
    NotPrepared,
    /// Image is temporarily unavailable.
    #[error("selected builder image is unavailable")]
    Unavailable,
    /// Image was retired.
    #[error("selected builder image is retired")]
    Retired,
    /// Requested resources exceed the image ceiling.
    #[error("agent.toml build resources exceed the builder image ceiling")]
    ResourceCeilingExceeded,
    /// Requested network exceeds the image ceiling.
    #[error("agent.toml build network exceeds the builder image ceiling")]
    NetworkCeilingExceeded,
    /// The source configuration has no version-2 build declaration.
    #[error("agent.toml does not contain a version-2 build declaration")]
    MissingBuild,
    /// The source configuration contains an invalid network profile.
    #[error("agent.toml build network profile is invalid")]
    InvalidNetwork,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> BuilderImage {
        BuilderImage {
            id: BuilderImageId::new(),
            key: BuilderKey::parse("rust").expect("key"),
            display_name: String::from("Rust builder"),
            image_reference: BuilderImageReference::parse(
                "registry.example/rust@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .expect("digest"),
            toolchains: vec![Toolchain {
                name: String::from("rust"),
                version: String::from("1.88.0"),
            }],
            architectures: vec![String::from("x86_64")],
            preparation: PreparationState::Ready,
            availability: AvailabilityState::Available,
            network_ceiling: BuildNetworkPolicy::Disabled,
            max_vcpus: 2,
            max_memory_mib: 512,
            dependency_policy: DependencyPolicy::VendoredOffline,
            provenance: BuilderProvenance {
                source: String::from("attestation://rust"),
                signature: None,
                sbom: None,
            },
            platform_policy_version: String::from("build/v1"),
        }
    }

    #[test]
    fn accepts_only_lowercase_digest_pinned_references() {
        assert!(BuilderImageReference::parse(
            "registry.example/rust@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        )
        .is_ok());
        assert!(BuilderImageReference::parse("registry.example/rust:latest").is_err());
        assert!(BuilderImageReference::parse(
            "registry.example/rust@sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        )
        .is_err());
    }

    #[test]
    fn enforces_availability_network_and_resource_policy() {
        let image = image();
        image.validate().expect("catalog entry is valid");
        assert!(
            image
                .validate_selection(BuildNetworkPolicy::Disabled, 2, 512)
                .is_ok()
        );
        assert_eq!(
            image.validate_selection(BuildNetworkPolicy::Egress, 1, 128),
            Err(BuilderSelectionError::NetworkCeilingExceeded)
        );
        assert_eq!(
            image.validate_selection(BuildNetworkPolicy::Disabled, 3, 128),
            Err(BuilderSelectionError::ResourceCeilingExceeded)
        );
    }
}
