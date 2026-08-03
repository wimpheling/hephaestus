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

    /// Returns the immutable digest embedded in this reference.
    ///
    /// # Errors
    ///
    /// Returns [`BuilderCatalogValueError::InvalidOciDigest`] if a value was
    /// deserialized without going through [`Self::parse`].
    pub fn digest(&self) -> Result<OciDigest, BuilderCatalogValueError> {
        let (repository, digest) = self
            .0
            .rsplit_once('@')
            .ok_or(BuilderCatalogValueError::InvalidOciDigest)?;
        if repository.is_empty()
            || repository
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        {
            return Err(BuilderCatalogValueError::InvalidOciDigest);
        }
        OciDigest::parse(digest)
    }
}

impl fmt::Display for BuilderImageReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
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
    /// Returns [`BuilderCatalogValueError::InvalidOciDigest`] for a malformed
    /// digest.
    pub fn parse(value: impl Into<String>) -> Result<Self, BuilderCatalogValueError> {
        let value = value.into();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(BuilderCatalogValueError::InvalidOciDigest);
        };
        if hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            Ok(Self(value))
        } else {
            Err(BuilderCatalogValueError::InvalidOciDigest)
        }
    }

    /// Returns the canonical digest string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OciDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for OciDigest {
    type Err = BuilderCatalogValueError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// A repository-relative Dockerfile or build-context path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BuilderSourcePath(String);

impl BuilderSourcePath {
    /// Parses a safe repository-relative POSIX path.
    ///
    /// # Errors
    ///
    /// Returns [`BuilderCatalogValueError::InvalidSourcePath`] for absolute,
    /// parent-traversing, control-containing, or oversized paths.
    pub fn parse(value: impl Into<String>) -> Result<Self, BuilderCatalogValueError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 1024
            && value == value.trim()
            && !value.starts_with('/')
            && !value.contains('\\')
            && !value
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte == 0)
            && (value == "."
                || value.split('/').all(|component| {
                    !component.is_empty() && component != "." && component != ".."
                }));
        valid
            .then_some(Self(value))
            .ok_or(BuilderCatalogValueError::InvalidSourcePath)
    }

    /// Returns the validated repository-relative path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BuilderSourcePath {
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

/// Stable identity for one project-owned builder definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectBuilderId(Uuid);

impl ProjectBuilderId {
    /// Creates a new project builder identity.
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

impl Default for ProjectBuilderId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ProjectBuilderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Lifecycle state for a project-owned OCI builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectBuilderStatus {
    /// Definition is immutable source configuration awaiting preparation.
    Draft,
    /// An external, policy-controlled builder is preparing the OCI image.
    Preparing,
    /// The OCI image has an immutable digest and verified provenance.
    Ready,
    /// Preparation failed and can be retried without changing the source.
    Failed,
    /// Definition is retained for history but cannot be selected.
    Retired,
}

/// Provenance recorded for a completed project-owned OCI builder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectBuilderProvenance {
    /// Immutable source revision used for the build.
    pub source_revision: String,
    /// Immutable digest of the submitted build context.
    pub context_digest: OciDigest,
    /// Reference to the external build attestation or audit record.
    pub attestation_reference: String,
    /// Optional SBOM or equivalent dependency inventory reference.
    pub sbom_reference: Option<String>,
}

impl ProjectBuilderProvenance {
    /// Validates provenance fields supplied by an external preparation worker.
    ///
    /// # Errors
    ///
    /// Returns [`BuilderCatalogValueError::InvalidProvenance`] when required
    /// attestation or source metadata is missing.
    pub fn validate(&self) -> Result<(), BuilderCatalogValueError> {
        OciDigest::parse(self.context_digest.as_str().to_owned())?;
        if !valid_source_revision(&self.source_revision)
            || self.attestation_reference.trim().is_empty()
            || self.attestation_reference.len() > 2048
        {
            return Err(BuilderCatalogValueError::InvalidProvenance);
        }
        if let Some(reference) = &self.sbom_reference
            && (reference.trim().is_empty() || reference.len() > 2048)
        {
            return Err(BuilderCatalogValueError::InvalidProvenance);
        }
        Ok(())
    }
}

/// Validated source and policy metadata for creating a project builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProjectBuilder {
    /// Stable project builder identity.
    pub id: ProjectBuilderId,
    /// Owning project identity.
    pub project_id: Uuid,
    /// Project-local builder key.
    pub key: BuilderKey,
    /// Human-readable builder name.
    pub display_name: String,
    /// Repository containing the Dockerfile and context.
    pub source_repository_id: Uuid,
    /// Immutable source revision containing the definition.
    pub source_revision: String,
    /// Repository-relative Dockerfile path.
    pub dockerfile_path: BuilderSourcePath,
    /// Repository-relative build context path.
    pub context_path: BuilderSourcePath,
    /// Immutable digest of the submitted context.
    pub context_digest: OciDigest,
    /// Exact digest-pinned platform image allowed as the base.
    pub approved_base_image: BuilderImageReference,
}

impl NewProjectBuilder {
    /// Validates metadata before it crosses the persistence boundary.
    ///
    /// # Errors
    ///
    /// Returns a stable value error for malformed or incomplete source data.
    pub fn validate(&self) -> Result<(), BuilderCatalogValueError> {
        if self.id.as_uuid().is_nil()
            || self.project_id.is_nil()
            || self.source_repository_id.is_nil()
        {
            return Err(BuilderCatalogValueError::InvalidProjectBuilderId);
        }
        if self.display_name.trim().is_empty() || self.display_name.len() > 200 {
            return Err(BuilderCatalogValueError::InvalidDisplayName);
        }
        BuilderKey::parse(self.key.as_str().to_owned())?;
        if !valid_source_revision(&self.source_revision) {
            return Err(BuilderCatalogValueError::InvalidSourceRevision);
        }
        BuilderSourcePath::parse(self.dockerfile_path.as_str().to_owned())?;
        BuilderSourcePath::parse(self.context_path.as_str().to_owned())?;
        OciDigest::parse(self.context_digest.as_str().to_owned())?;
        BuilderImageReference::parse(self.approved_base_image.as_str().to_owned())?;
        Ok(())
    }
}

/// Persisted project-owned builder definition and lifecycle metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectBuilderDefinition {
    /// Stable project builder identity.
    pub id: ProjectBuilderId,
    /// Owning project identity.
    pub project_id: Uuid,
    /// Project-local builder key.
    pub key: BuilderKey,
    /// Human-readable builder name.
    pub display_name: String,
    /// Repository containing the Dockerfile and context.
    pub source_repository_id: Uuid,
    /// Immutable source revision containing the definition.
    pub source_revision: String,
    /// Repository-relative Dockerfile path.
    pub dockerfile_path: BuilderSourcePath,
    /// Repository-relative build context path.
    pub context_path: BuilderSourcePath,
    /// Immutable digest of the submitted context.
    pub context_digest: OciDigest,
    /// Exact digest-pinned platform image allowed as the base.
    pub approved_base_image: BuilderImageReference,
    /// Current durable lifecycle state.
    pub status: ProjectBuilderStatus,
    /// Immutable digest-pinned output image reference after completion.
    pub oci_image_reference: Option<BuilderImageReference>,
    /// Output image digest copied from the immutable reference.
    pub oci_image_digest: Option<OciDigest>,
    /// Attestation and source provenance after completion.
    pub provenance: Option<ProjectBuilderProvenance>,
    /// Operator-visible preparation failure, if any.
    pub failure_reason: Option<String>,
    /// Creation timestamp.
    pub created_at: time::OffsetDateTime,
    /// Last lifecycle update timestamp.
    pub updated_at: time::OffsetDateTime,
}

impl ProjectBuilderDefinition {
    /// Validates all durable lifecycle invariants.
    ///
    /// # Errors
    ///
    /// Returns a stable value error for malformed metadata or an inconsistent
    /// lifecycle projection.
    pub fn validate(&self) -> Result<(), BuilderCatalogValueError> {
        NewProjectBuilder {
            id: self.id,
            project_id: self.project_id,
            key: self.key.clone(),
            display_name: self.display_name.clone(),
            source_repository_id: self.source_repository_id,
            source_revision: self.source_revision.clone(),
            dockerfile_path: self.dockerfile_path.clone(),
            context_path: self.context_path.clone(),
            context_digest: self.context_digest.clone(),
            approved_base_image: self.approved_base_image.clone(),
        }
        .validate()?;

        match self.status {
            ProjectBuilderStatus::Ready => {
                let output_reference = self
                    .oci_image_reference
                    .as_ref()
                    .ok_or(BuilderCatalogValueError::InvalidProjectBuilderState)?;
                let output_digest = self
                    .oci_image_digest
                    .as_ref()
                    .ok_or(BuilderCatalogValueError::InvalidProjectBuilderState)?;
                if output_reference.digest()? != *output_digest {
                    return Err(BuilderCatalogValueError::InvalidProjectBuilderState);
                }
                let provenance = self
                    .provenance
                    .as_ref()
                    .ok_or(BuilderCatalogValueError::InvalidProjectBuilderState)?;
                provenance.validate()?;
                if provenance.source_revision != self.source_revision
                    || provenance.context_digest != self.context_digest
                {
                    return Err(BuilderCatalogValueError::InvalidProjectBuilderState);
                }
                if self.failure_reason.is_some() {
                    return Err(BuilderCatalogValueError::InvalidProjectBuilderState);
                }
            }
            ProjectBuilderStatus::Failed => {
                if self.failure_reason.as_deref().is_none_or(str::is_empty)
                    || self
                        .failure_reason
                        .as_ref()
                        .is_some_and(|reason| reason.len() > 2048)
                    || self.oci_image_reference.is_some()
                    || self.oci_image_digest.is_some()
                    || self.provenance.is_some()
                {
                    return Err(BuilderCatalogValueError::InvalidProjectBuilderState);
                }
            }
            ProjectBuilderStatus::Draft | ProjectBuilderStatus::Preparing => {
                if self.failure_reason.is_some()
                    || self.oci_image_reference.is_some()
                    || self.oci_image_digest.is_some()
                    || self.provenance.is_some()
                {
                    return Err(BuilderCatalogValueError::InvalidProjectBuilderState);
                }
            }
            ProjectBuilderStatus::Retired => {
                if let (Some(output_reference), Some(output_digest), Some(provenance)) = (
                    self.oci_image_reference.as_ref(),
                    self.oci_image_digest.as_ref(),
                    self.provenance.as_ref(),
                ) {
                    if output_reference.digest()? != *output_digest
                        || provenance.source_revision != self.source_revision
                        || provenance.context_digest != self.context_digest
                        || self.failure_reason.is_some()
                    {
                        return Err(BuilderCatalogValueError::InvalidProjectBuilderState);
                    }
                    provenance.validate()?;
                } else if self.oci_image_reference.is_some()
                    || self.oci_image_digest.is_some()
                    || self.provenance.is_some()
                    || self
                        .failure_reason
                        .as_ref()
                        .is_some_and(|reason| reason.trim().is_empty() || reason.len() > 2048)
                {
                    return Err(BuilderCatalogValueError::InvalidProjectBuilderState);
                }
            }
        }
        Ok(())
    }

    /// Transitions a draft or failed definition into preparation.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectBuilderLifecycleError::InvalidTransition`] when the
    /// definition is not retryable.
    pub fn begin_preparation(mut self) -> Result<Self, ProjectBuilderLifecycleError> {
        match self.status {
            ProjectBuilderStatus::Draft | ProjectBuilderStatus::Failed => {
                self.status = ProjectBuilderStatus::Preparing;
                self.failure_reason = None;
                Ok(self)
            }
            ProjectBuilderStatus::Preparing
            | ProjectBuilderStatus::Ready
            | ProjectBuilderStatus::Retired => Err(ProjectBuilderLifecycleError::InvalidTransition),
        }
    }

    /// Completes preparation with an immutable output and matching provenance.
    ///
    /// # Errors
    ///
    /// Returns a lifecycle or value error if the definition is not preparing,
    /// or if output provenance does not match its immutable source.
    pub fn complete(
        mut self,
        output_reference: BuilderImageReference,
        output_digest: OciDigest,
        provenance: ProjectBuilderProvenance,
    ) -> Result<Self, ProjectBuilderLifecycleError> {
        if self.status != ProjectBuilderStatus::Preparing {
            return Err(ProjectBuilderLifecycleError::InvalidTransition);
        }
        if output_reference
            .digest()
            .map_err(ProjectBuilderLifecycleError::InvalidValue)?
            != output_digest
        {
            return Err(ProjectBuilderLifecycleError::InvalidValue(
                BuilderCatalogValueError::InvalidProjectBuilderState,
            ));
        }
        provenance
            .validate()
            .map_err(ProjectBuilderLifecycleError::InvalidValue)?;
        if provenance.source_revision != self.source_revision
            || provenance.context_digest != self.context_digest
        {
            return Err(ProjectBuilderLifecycleError::InvalidValue(
                BuilderCatalogValueError::InvalidProvenance,
            ));
        }
        self.status = ProjectBuilderStatus::Ready;
        self.oci_image_reference = Some(output_reference);
        self.oci_image_digest = Some(output_digest);
        self.provenance = Some(provenance);
        self.failure_reason = None;
        Ok(self)
    }

    /// Records a preparation failure without changing immutable source data.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectBuilderLifecycleError::InvalidTransition`] when the
    /// definition is not currently preparing.
    pub fn fail(mut self, reason: impl Into<String>) -> Result<Self, ProjectBuilderLifecycleError> {
        if self.status != ProjectBuilderStatus::Preparing {
            return Err(ProjectBuilderLifecycleError::InvalidTransition);
        }
        let reason = reason.into();
        if reason.trim().is_empty() || reason.len() > 2048 {
            return Err(ProjectBuilderLifecycleError::InvalidValue(
                BuilderCatalogValueError::InvalidFailureReason,
            ));
        }
        self.status = ProjectBuilderStatus::Failed;
        self.failure_reason = Some(reason);
        Ok(self)
    }

    /// Retires a definition while retaining its historical metadata.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectBuilderLifecycleError::InvalidTransition`] if it is
    /// already retired.
    pub fn retire(mut self) -> Result<Self, ProjectBuilderLifecycleError> {
        if self.status == ProjectBuilderStatus::Retired {
            return Err(ProjectBuilderLifecycleError::InvalidTransition);
        }
        self.status = ProjectBuilderStatus::Retired;
        Ok(self)
    }
}

/// Invalid project-builder lifecycle transition.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectBuilderLifecycleError {
    /// The requested operation is not valid for the current state.
    #[error("project builder lifecycle transition is invalid")]
    InvalidTransition,
    /// The lifecycle payload violates a domain invariant.
    #[error("project builder lifecycle payload is invalid: {0}")]
    InvalidValue(#[source] BuilderCatalogValueError),
}

fn valid_source_revision(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
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
    /// A project builder identity is missing or nil.
    #[error("project builder identifiers must be non-nil")]
    InvalidProjectBuilderId,
    /// A project builder source revision is not an immutable commit digest.
    #[error(
        "project builder source revision must be a lowercase 40- or 64-character commit digest"
    )]
    InvalidSourceRevision,
    /// A Dockerfile or context path escapes the repository.
    #[error("project builder source path must be repository-relative and safe")]
    InvalidSourcePath,
    /// An OCI output or context digest is malformed.
    #[error("OCI digest must be a lowercase sha256 digest")]
    InvalidOciDigest,
    /// A project builder lifecycle projection is inconsistent.
    #[error("project builder lifecycle projection is inconsistent")]
    InvalidProjectBuilderState,
    /// A project builder completion lacks valid provenance.
    #[error("project builder provenance is invalid")]
    InvalidProvenance,
    /// A project builder failure reason is missing or too large.
    #[error("project builder failure reason is invalid")]
    InvalidFailureReason,
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

    fn project_builder() -> ProjectBuilderDefinition {
        ProjectBuilderDefinition {
            id: ProjectBuilderId::new(),
            project_id: Uuid::new_v4(),
            key: BuilderKey::parse("custom").expect("key"),
            display_name: String::from("Custom builder"),
            source_repository_id: Uuid::new_v4(),
            source_revision: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            dockerfile_path: BuilderSourcePath::parse("builders/Dockerfile").expect("path"),
            context_path: BuilderSourcePath::parse(".").expect("context"),
            context_digest: OciDigest::parse(
                "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )
            .expect("context digest"),
            approved_base_image: BuilderImageReference::parse(
                "registry.example/ubuntu@sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            )
            .expect("base image"),
            status: ProjectBuilderStatus::Draft,
            oci_image_reference: None,
            oci_image_digest: None,
            provenance: None,
            failure_reason: None,
            created_at: time::OffsetDateTime::UNIX_EPOCH,
            updated_at: time::OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn project_builder_lifecycle_preserves_immutable_output_provenance() {
        let draft = project_builder();
        draft.validate().expect("valid draft");
        let preparing = draft.begin_preparation().expect("preparing");
        let output_digest = OciDigest::parse(
            "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        )
        .expect("output digest");
        let ready = preparing
            .complete(
                BuilderImageReference::parse(format!("registry.example/custom@{output_digest}"))
                    .expect("output reference"),
                output_digest.clone(),
                ProjectBuilderProvenance {
                    source_revision: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
                    context_digest: OciDigest::parse(
                        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    )
                    .expect("context digest"),
                    attestation_reference: String::from("attestation://test-build"),
                    sbom_reference: None,
                },
            )
            .expect("ready");
        ready.validate().expect("valid ready");
        assert_eq!(ready.oci_image_digest, Some(output_digest));
        let retired = ready.retire().expect("retired");
        retired.validate().expect("valid retired");
        assert!(retired.oci_image_reference.is_some());
        assert!(retired.provenance.is_some());
    }

    #[test]
    fn failed_project_builder_can_retry_without_changing_source() {
        let draft = project_builder();
        let preparing = draft.begin_preparation().expect("preparing");
        let failed = preparing.fail("OCI preparation failed").expect("failed");
        failed.validate().expect("valid failed state");
        let retried = failed.begin_preparation().expect("retry");
        assert_eq!(retried.status, ProjectBuilderStatus::Preparing);
        assert_eq!(
            retried.source_revision,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert!(retried.failure_reason.is_none());
    }

    #[test]
    fn source_paths_cannot_escape_the_repository() {
        assert!(BuilderSourcePath::parse(".").is_ok());
        assert!(BuilderSourcePath::parse("builders/Dockerfile").is_ok());
        assert!(BuilderSourcePath::parse("../Dockerfile").is_err());
        assert!(BuilderSourcePath::parse("/tmp/Dockerfile").is_err());
        assert!(BuilderSourcePath::parse("builders\\Dockerfile").is_err());
    }
}
