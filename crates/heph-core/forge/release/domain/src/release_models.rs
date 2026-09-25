use forge_domain::{CommitSha, GitRef, RepositoryId};
use runtime_types::{ReleaseAgentId, ReleaseId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    AgentFamilyId, AgentKey, ArtifactPath, BuildRequestId, ContentHash, ParameterDeclaration,
    ReleaseArtifactId, ReleaseValueError, ReleaseVersion,
};
use super::{BuildState, ReleaseState};

/// One immutable source-repository-owned agent family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFamily {
    /// Stable family identifier.
    pub id: AgentFamilyId,
    /// Owning source repository.
    pub repository_id: RepositoryId,
    /// Stable repository-scoped exported key.
    pub agent_key: AgentKey,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// Complete exact isolated-build request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildRequest {
    /// Durable build identity.
    pub id: BuildRequestId,
    /// Source repository.
    pub repository_id: RepositoryId,
    /// Exact source commit.
    pub source_commit: CommitSha,
    /// Source ref provenance.
    pub source_ref: GitRef,
    /// Normalized build-definition hash.
    pub build_definition_hash: ContentHash,
    /// Current lifecycle.
    pub state: BuildState,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// Immutable or draft release metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Release identifier.
    pub id: ReleaseId,
    /// Source repository.
    pub repository_id: RepositoryId,
    /// Repository-selected version.
    pub version: ReleaseVersion,
    /// Exact source commit.
    pub source_commit: CommitSha,
    /// Exact source ref provenance.
    pub source_ref: GitRef,
    /// Build that produced every artifact.
    pub build_request_id: BuildRequestId,
    /// Hash of the normalized source configuration.
    pub configuration_hash: ContentHash,
    /// Complete artifact-manifest hash.
    pub manifest_hash: ContentHash,
    /// Lifecycle.
    pub state: ReleaseState,
    /// Publication time, once frozen.
    pub published_at: Option<OffsetDateTime>,
}

/// Release artifact kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// Executable runtime payload.
    Executable,
    /// Immutable data file.
    File,
    /// Complete release manifest.
    Manifest,
    /// Build log stream.
    BuildLog,
}

/// Immutable artifact metadata and opaque storage identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseArtifact {
    /// Artifact identifier.
    pub id: ReleaseArtifactId,
    /// Parent release.
    pub release_id: ReleaseId,
    /// Normalized path under `/release`.
    pub path: ArtifactPath,
    /// Artifact kind.
    pub kind: ArtifactKind,
    /// Unix permission bits with file-type bits excluded.
    pub mode: u16,
    /// Content hash.
    pub content_hash: ContentHash,
    /// Byte length.
    pub size_bytes: u64,
    /// Bounded media type.
    pub media_type: String,
    /// Opaque canonical storage key.
    pub storage_key: Uuid,
}

/// Immutable release-owned runtime contract for one exported agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAgent {
    /// Export identifier.
    pub id: ReleaseAgentId,
    /// Parent release.
    pub release_id: ReleaseId,
    /// Stable compatible family.
    pub family_id: AgentFamilyId,
    /// Stable exported key.
    pub agent_key: AgentKey,
    /// Mutable human-readable display name.
    pub display_name: String,
    /// Release-relative executable path.
    pub executable: ArtifactPath,
    /// Immutable arguments.
    pub arguments: Vec<String>,
    /// Release-relative working directory.
    pub working_directory: ArtifactPath,
    /// Exact immutable OCI image reference selected for guest execution.
    pub image_reference: String,
    /// Whether one persistent volume is required per consuming instance.
    pub requires_state: bool,
    /// Normalized release-owned policy ceiling.
    pub policy_ceiling: RuntimePolicy,
    /// Typed parameter declarations.
    pub parameters: Vec<ParameterDeclaration>,
    /// Optional update hook.
    pub update_hook: Option<UpdateHook>,
}

/// Bounded compute and network policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimePolicy {
    /// Virtual CPU count.
    pub vcpus: u8,
    /// Guest memory in mebibytes.
    pub memory_mib: u32,
    /// Provider-neutral network ceiling.
    pub network: NetworkAccess,
}

impl RuntimePolicy {
    /// Validates a consumer selection against a release ceiling and platform
    /// policy.
    ///
    /// # Errors
    ///
    /// Returns [`ReleaseValueError::PolicyBroadening`] when any dimension
    /// exceeds either ceiling.
    pub fn resolve(
        release: &Self,
        project: &Self,
        platform: &Self,
    ) -> Result<Self, ReleaseValueError> {
        if project.vcpus > release.vcpus
            || project.memory_mib > release.memory_mib
            || project.network.broader_than(release.network)
            || project.vcpus > platform.vcpus
            || project.memory_mib > platform.memory_mib
            || project.network.broader_than(platform.network)
        {
            return Err(ReleaseValueError::PolicyBroadening);
        }
        Ok(project.clone())
    }
}

/// Ordered provider-neutral network restriction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkAccess {
    /// No guest networking.
    Disabled,
    /// Only host broker connectivity.
    BrokerOnly,
    /// Constrained external egress.
    Egress,
}

impl NetworkAccess {
    const fn rank(self) -> u8 {
        match self {
            Self::Disabled => 0,
            Self::BrokerOnly => 1,
            Self::Egress => 2,
        }
    }

    const fn broader_than(self, other: Self) -> bool {
        self.rank() > other.rank()
    }
}

/// Candidate-release update-hook contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateHook {
    /// Release-relative executable.
    pub executable: ArtifactPath,
    /// Fixed arguments.
    pub arguments: Vec<String>,
    /// Wall-clock timeout in seconds.
    pub timeout_seconds: u32,
    /// Update-specific policy within the release ceiling.
    pub policy: RuntimePolicy,
}
