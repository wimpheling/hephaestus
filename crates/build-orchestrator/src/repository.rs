//! Provider-neutral durable build persistence ports.
// The port is intentionally data-heavy; field-level docs would obscure the
// persistence boundary and duplicate the domain model documentation.
#![allow(missing_docs)]

use agent_config::BuildConfig;
use async_trait::async_trait;
use release_domain::{BuildRequestId, ReleaseAgentId, ReleaseId, ReleaseVersion};
use serde_json::Value;
use uuid::Uuid;
use vm_trait::VmExit;

/// Durable input selected for an exact build request.
#[derive(Clone)]
pub struct BuildInput {
    pub id: BuildRequestId,
    pub repository_id: Uuid,
    pub source_commit: String,
    pub source_ref: String,
    pub build: BuildConfig,
    /// Exact immutable OCI image reference resolved before this request was
    /// persisted. Worker execution never resolves mutable catalog keys.
    pub image_reference: String,
}

/// Durable identity allocated while claiming a build.
pub struct ClaimedBuild {
    pub input: BuildInput,
    pub release_id: ReleaseId,
    pub release_agent_id: ReleaseAgentId,
    pub release_version: ReleaseVersion,
}

/// Durable execution selected for restart recovery.
pub struct RecoverableBuild {
    pub id: BuildRequestId,
    pub vm_id: String,
}

/// Durable finalization state.
pub struct FinalizationBuild {
    pub state: String,
    pub claimed: ClaimedBuild,
    pub artifact_manifest: Option<Value>,
}

/// Persistence failures exposed by the provider-neutral build port.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BuildRepositoryError {
    #[error("build request is unavailable")]
    Unavailable,
    #[error("build request is already claimed")]
    AlreadyClaimed,
    #[error("build request is not authorized")]
    Unauthorized,
    #[error("build authorization failed")]
    Authorization,
    #[error("stored build state is invalid")]
    InvalidData,
    #[error("build repository operation failed: {0}")]
    Storage(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Provider-neutral durable build persistence boundary.
#[async_trait]
pub trait BuildRepository: Send + Sync + 'static {
    /// Archives the active failed attempt and resets the execution for a
    /// trusted retry worker.
    async fn reset_for_retry(&self, id: BuildRequestId) -> Result<(), BuildRepositoryError>;
    /// Claims a verification execution without changing the immutable build.
    async fn claim_verification(
        &self,
        id: BuildRequestId,
    ) -> Result<ClaimedBuild, BuildRepositoryError>;
    /// Stores the verification result and returns whether manifests matched.
    async fn complete_verification(
        &self,
        id: BuildRequestId,
        actual_manifest: &Value,
    ) -> Result<bool, BuildRepositoryError>;
    /// Records a verification execution failure.
    async fn fail_verification(
        &self,
        id: BuildRequestId,
        code: &str,
    ) -> Result<(), BuildRepositoryError>;
    async fn recoverable(&self) -> Result<Vec<RecoverableBuild>, BuildRepositoryError>;
    async fn finalizing(&self) -> Result<Vec<BuildRequestId>, BuildRepositoryError>;
    /// Resets an execution after provider cleanup confirms no guest remains.
    async fn reset_after_cleanup(&self, id: BuildRequestId) -> Result<(), BuildRepositoryError>;
    async fn completed(
        &self,
        id: BuildRequestId,
    ) -> Result<Option<(ReleaseId, ReleaseAgentId, ReleaseVersion, usize)>, BuildRepositoryError>;
    async fn finalization(
        &self,
        id: BuildRequestId,
    ) -> Result<Option<FinalizationBuild>, BuildRepositoryError>;
    async fn claim(&self, id: BuildRequestId) -> Result<ClaimedBuild, BuildRepositoryError>;
    async fn mark_running(&self, id: BuildRequestId) -> Result<(), BuildRepositoryError>;
    async fn mark_sealed(
        &self,
        id: BuildRequestId,
        exit: &VmExit,
        logs: &[Value],
        metrics: &[Value],
    ) -> Result<(), BuildRepositoryError>;
    async fn mark_imported(
        &self,
        id: BuildRequestId,
        artifacts: &[Value],
    ) -> Result<(), BuildRepositoryError>;
    async fn mark_drafted(&self, id: BuildRequestId) -> Result<(), BuildRepositoryError>;
    async fn fail(
        &self,
        id: BuildRequestId,
        code: &str,
        exit_code: Option<i32>,
        exit_signal: Option<i32>,
        logs: &[Value],
        metrics: &[Value],
    ) -> Result<(), BuildRepositoryError>;
}
