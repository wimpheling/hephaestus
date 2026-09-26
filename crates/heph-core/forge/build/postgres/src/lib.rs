//! `PostgreSQL` persistence adapter for isolated build execution.
//!
//! Build execution ports are being staged here while filesystem, Git, VM, and
//! artifact effects remain owned by `build-orchestrator`.

mod execution;
mod lifecycle;
mod model;
mod verification;

use async_trait::async_trait;
use build_orchestrator::{
    BuildRepository, BuildRepositoryError, ClaimedBuild, FinalizationBuild, RecoverableBuild,
};
use release_domain::{BuildRequestId, ReleaseAgentId, ReleaseId, ReleaseVersion};
use serde_json::Value;
use sqlx::PgPool;
use vm_trait::VmExit;

/// `PostgreSQL` implementation of the provider-neutral build persistence port.
#[derive(Clone)]
pub struct PgBuildRepository {
    pool: PgPool,
}

impl PgBuildRepository {
    /// Creates a repository backed by an existing `PostgreSQL` pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl BuildRepository for PgBuildRepository {
    async fn image_reference(&self, id: BuildRequestId) -> Result<String, BuildRepositoryError> {
        self.image_reference_impl(id).await
    }

    async fn reset_for_retry(&self, id: BuildRequestId) -> Result<(), BuildRepositoryError> {
        self.reset_for_retry_impl(id).await
    }

    async fn claim_verification(
        &self,
        id: BuildRequestId,
    ) -> Result<ClaimedBuild, BuildRepositoryError> {
        self.claim_verification_impl(id).await
    }

    async fn complete_verification(
        &self,
        id: BuildRequestId,
        actual_manifest: &Value,
    ) -> Result<bool, BuildRepositoryError> {
        self.complete_verification_impl(id, actual_manifest).await
    }

    async fn fail_verification(
        &self,
        id: BuildRequestId,
        code: &str,
    ) -> Result<(), BuildRepositoryError> {
        self.fail_verification_impl(id, code).await
    }

    async fn recoverable(&self) -> Result<Vec<RecoverableBuild>, BuildRepositoryError> {
        self.recoverable_impl().await
    }

    async fn finalizing(&self) -> Result<Vec<BuildRequestId>, BuildRepositoryError> {
        self.finalizing_impl().await
    }

    async fn reset_after_cleanup(&self, id: BuildRequestId) -> Result<(), BuildRepositoryError> {
        self.reset_after_cleanup_impl(id).await
    }

    async fn completed(
        &self,
        id: BuildRequestId,
    ) -> Result<Option<(ReleaseId, ReleaseAgentId, ReleaseVersion, usize)>, BuildRepositoryError>
    {
        self.completed_impl(id).await
    }

    async fn finalization(
        &self,
        id: BuildRequestId,
    ) -> Result<Option<FinalizationBuild>, BuildRepositoryError> {
        self.finalization_impl(id).await
    }

    async fn claim(&self, id: BuildRequestId) -> Result<ClaimedBuild, BuildRepositoryError> {
        self.claim_impl(id).await
    }

    async fn mark_running(&self, id: BuildRequestId) -> Result<(), BuildRepositoryError> {
        self.mark_running_impl(id).await
    }

    async fn mark_sealed(
        &self,
        id: BuildRequestId,
        exit: &VmExit,
        logs: &[Value],
        metrics: &[Value],
    ) -> Result<(), BuildRepositoryError> {
        self.mark_sealed_impl(id, exit, logs, metrics).await
    }

    async fn mark_imported(
        &self,
        id: BuildRequestId,
        artifacts: &[Value],
    ) -> Result<(), BuildRepositoryError> {
        self.mark_imported_impl(id, artifacts).await
    }

    async fn mark_drafted(&self, id: BuildRequestId) -> Result<(), BuildRepositoryError> {
        self.mark_drafted_impl(id).await
    }

    async fn fail(
        &self,
        id: BuildRequestId,
        code: &str,
        exit_code: Option<i32>,
        exit_signal: Option<i32>,
        logs: &[Value],
        metrics: &[Value],
    ) -> Result<(), BuildRepositoryError> {
        self.fail_impl(id, code, exit_code, exit_signal, logs, metrics)
            .await
    }
}
