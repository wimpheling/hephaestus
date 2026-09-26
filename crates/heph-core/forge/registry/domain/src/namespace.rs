//! Registry namespace ownership contracts.
use crate::{
    RegistryValueError,
    errors::{canonical_project_id, canonical_project_image_id, canonical_release_agent_id},
};
use builder_catalog_domain::OciImageId;
use forge_domain::ProjectId;
use runtime_types::ReleaseAgentId;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use uuid::Uuid;

/// A stable identity for one durable registry publication intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PublicationIntentId(Uuid);

impl PublicationIntentId {
    /// Creates a new publication-intent identity.
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

impl Default for PublicationIntentId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PublicationIntentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for PublicationIntentId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// A bounded stable key for a platform-owned image namespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PlatformImageKey(String);

impl PlatformImageKey {
    /// Parses a canonical platform image key.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryValueError::InvalidPlatformImageKey`] when the key
    /// is not a bounded lowercase identifier.
    pub fn parse(value: impl Into<String>) -> Result<Self, RegistryValueError> {
        let value = value.into();
        let valid = (1..=64).contains(&value.len())
            && value.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || ((byte == b'_' || byte == b'-') && index > 0)
            });
        valid
            .then_some(Self(value))
            .ok_or(RegistryValueError::InvalidPlatformImageKey)
    }

    /// Returns the canonical key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PlatformImageKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for PlatformImageKey {
    type Error = RegistryValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<PlatformImageKey> for String {
    fn from(value: PlatformImageKey) -> Self {
        value.0
    }
}

/// The durable resource that exclusively owns one registry namespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RegistryOwner {
    /// One platform-owned image selected by its stable key.
    PlatformImage {
        /// Stable platform image key.
        image_key: PlatformImageKey,
    },
    /// One project-owned repository OCI image.
    RepositoryOciImage {
        /// Owning project.
        project_id: ProjectId,
        /// Stable image identity.
        image_id: OciImageId,
    },
    /// One project-owned release agent.
    ReleaseAgent {
        /// Owning project.
        project_id: ProjectId,
        /// Stable release-agent identity.
        release_agent_id: ReleaseAgentId,
    },
}

impl RegistryOwner {
    /// Returns this owner's canonical repository path.
    #[must_use]
    pub fn repository_path(&self) -> String {
        match self {
            Self::PlatformImage { image_key } => {
                format!("platform/images/{image_key}")
            }
            Self::RepositoryOciImage {
                project_id,
                image_id,
            } => format!("projects/{project_id}/repository-images/{image_id}"),
            Self::ReleaseAgent {
                project_id,
                release_agent_id,
            } => format!("projects/{project_id}/release-agents/{release_agent_id}"),
        }
    }
}

/// A canonical registry repository path with no mutable human-name component.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RegistryNamespace {
    path: String,
    owner: RegistryOwner,
}

impl RegistryNamespace {
    /// Derives the sole canonical path for an owner.
    #[must_use]
    pub fn for_owner(owner: RegistryOwner) -> Self {
        let path = owner.repository_path();
        Self { path, owner }
    }

    /// Parses one supported canonical namespace path.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryValueError::InvalidNamespace`] when the path is not
    /// one of the supported paths or contains a non-canonical UUID.
    pub fn parse(value: impl Into<String>) -> Result<Self, RegistryValueError> {
        let value = value.into();
        let parts = value.split('/').collect::<Vec<_>>();
        let owner = match parts.as_slice() {
            ["platform", "images", image_key] => RegistryOwner::PlatformImage {
                image_key: PlatformImageKey::parse((*image_key).to_owned())?,
            },
            ["projects", project_id, "repository-images", image_id] => {
                RegistryOwner::RepositoryOciImage {
                    project_id: canonical_project_id(project_id)?,
                    image_id: canonical_project_image_id(image_id)?,
                }
            }
            ["projects", project_id, "release-agents", release_agent_id] => {
                RegistryOwner::ReleaseAgent {
                    project_id: canonical_project_id(project_id)?,
                    release_agent_id: canonical_release_agent_id(release_agent_id)?,
                }
            }
            _ => return Err(RegistryValueError::InvalidNamespace),
        };
        let namespace = Self::for_owner(owner);
        (namespace.path == value)
            .then_some(namespace)
            .ok_or(RegistryValueError::InvalidNamespace)
    }

    /// Returns the canonical slash-separated repository path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.path
    }

    /// Returns the durable owner encoded by this namespace.
    #[must_use]
    pub const fn owner(&self) -> &RegistryOwner {
        &self.owner
    }
}

impl fmt::Display for RegistryNamespace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.path.fmt(formatter)
    }
}

impl FromStr for RegistryNamespace {
    type Err = RegistryValueError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value.to_owned())
    }
}

/// Stable ownership of one exact namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceClaim {
    namespace: RegistryNamespace,
    owner: RegistryOwner,
}

impl NamespaceClaim {
    /// Creates the exclusive namespace claim for an owner.
    #[must_use]
    pub fn new(owner: RegistryOwner) -> Self {
        let namespace = RegistryNamespace::for_owner(owner.clone());
        Self { namespace, owner }
    }

    /// Returns the claimed namespace.
    #[must_use]
    pub const fn namespace(&self) -> &RegistryNamespace {
        &self.namespace
    }

    /// Returns the owning resource.
    #[must_use]
    pub const fn owner(&self) -> &RegistryOwner {
        &self.owner
    }

    /// Confirms that an exact namespace belongs to this claim.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryOwnershipError::NamespaceMismatch`] for another
    /// project or resource namespace.
    pub fn assert_owns(&self, namespace: &RegistryNamespace) -> Result<(), RegistryOwnershipError> {
        (self.namespace == *namespace)
            .then_some(())
            .ok_or(RegistryOwnershipError::NamespaceMismatch)
    }
}

/// Registry-authority validation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryOwnershipError {
    /// The namespace is not the one derived from the durable owner.
    #[error("registry namespace does not belong to this durable owner")]
    NamespaceMismatch,
}
