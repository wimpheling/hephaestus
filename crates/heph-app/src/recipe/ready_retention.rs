//! Current-manager Ready retention with actual closed-admission native custody.
//!
//! This one-volume service performs no VM drain, root bootstrap or source actor
//! reconstruction. Outstanding consumer cleanup stays with its actual scoped
//! runtime owner; unsupported or unproved histories remain held.
mod completion;
mod validation;

use heph_runtime::VmProvider;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, OwnedReadyRetentionRepository, ResourceProgress, RetainOwnedReady,
};
use std::sync::Arc;
use volume_local::LocalVolumeStore;
use volume_trait::{OwnedProvisioningClaim, VolumeError, VolumeReadyRetentionRepository};

/// Backend retention errors preserve definite commit versus unknown/held results.
#[derive(Debug, thiserror::Error)]
pub enum ReadyRetentionError {
    /// Current manager command or atomic disposition was denied.
    #[error(transparent)]
    Deployment(#[from] DeploymentError),
    /// Original closed context or physical custody is unavailable/contradictory.
    #[error(transparent)]
    Volume(#[from] VolumeError),
    /// Actual configured provider scope is unsupported or differs from durable pins.
    #[error("Ready retention provider scope is unavailable or mismatched")]
    Scope,
    /// Full original preparation/operation/closure comparison failed before IO.
    #[error("Ready retention original birth does not match")]
    Correlation,
    /// Protected completion ended with an unknown transaction outcome.
    #[error("Ready retention completion is unknown; inspect with fresh management")]
    CompletionUnknown,
    /// Actor commit succeeded but a subsequent native validation failed.
    #[error("Ready retention committed; native custody requires inspection")]
    CommittedCustodyChanged {
        /// Positive committed progress; never reported as a rolled-back Remove.
        progress: ResourceProgress,
    },
}
/// One-volume owning coordinator; actual provider and journal family stay retained.
pub struct ReadyRetentionCoordinator {
    actor: Arc<dyn OwnedReadyRetentionRepository>,
    worker: Arc<dyn VolumeReadyRetentionRepository>,
    local: Arc<LocalVolumeStore>,
    provider: Arc<dyn VmProvider>,
}
impl ReadyRetentionCoordinator {
    /// Injects actual trusted provider clone, Local store and separated role ports.
    ///
    /// Scope comparison does not drain guests; the worker port requires positive
    /// persisted scoped cleanup evidence from each actual owning runtime.
    #[must_use]
    pub const fn new(
        actor: Arc<dyn OwnedReadyRetentionRepository>,
        worker: Arc<dyn VolumeReadyRetentionRepository>,
        local: Arc<LocalVolumeStore>,
        provider: Arc<dyn VmProvider>,
    ) -> Self {
        Self {
            actor,
            worker,
            local,
            provider,
        }
    }
    /// Retains one positive Ready birth under a current manager's real Remove.
    ///
    /// Commits closure first, then requires global scoped quiescence before fsck.
    /// Completed birth replay performs no physical IO, release or generation reset.
    /// Actual non-clone custody moves into protected completion before any record
    /// or actor commit wait; caller cancellation cannot drop it during those waits.
    ///
    /// # Errors
    /// Lost management, foreign scope/birth, unsupported history, held consumers,
    /// dirty/replaced backing or ambiguous completion remains held and CLOSED.
    pub async fn retain(
        &self,
        identity: &AuthenticatedIdentity,
        request: &RetainOwnedReady,
        claim: &OwnedProvisioningClaim,
    ) -> Result<ResourceProgress, ReadyRetentionError> {
        Box::pin(self.retain_inner(identity, request, claim)).await
    }
    async fn retain_inner(
        &self,
        identity: &AuthenticatedIdentity,
        request: &RetainOwnedReady,
        claim: &OwnedProvisioningClaim,
    ) -> Result<ResourceProgress, ReadyRetentionError> {
        let scope = validation::scope(self.provider.as_ref())?;
        let plan = self
            .actor
            .begin_owned_ready_retention(identity, request)
            .await?;
        validation::plan(&plan, claim, &request.target, &scope)?;
        if let Some(progress) = plan.completed {
            return Ok(progress);
        }
        let context = self
            .worker
            .ready_retention_context(claim, plan.closure_id)
            .await?;
        validation::context(&plan, claim, &context, &scope)?;
        let custody = self
            .local
            .observe_owned_ready_retention(self.worker.as_ref(), &context)
            .await?;
        let actor = self.actor.clone();
        let worker = self.worker.clone();
        let provider = self.provider.clone();
        let identity = identity.clone();
        let request = request.clone();
        tokio::spawn(async move {
            completion::commit(actor, worker, provider, identity, request, custody).await
        })
        .await
        .map_err(|_| ReadyRetentionError::CompletionUnknown)?
    }
}
