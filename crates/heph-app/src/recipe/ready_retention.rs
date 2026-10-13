//! Current-manager Ready retention with actual closed-admission native custody.
//!
//! This one-volume service performs no VM drain, root bootstrap or source actor
//! reconstruction. Outstanding consumer cleanup stays with its actual scoped
//! runtime owner; unsupported or unproved histories remain held.
mod completion;
mod handover;
pub(crate) use handover::Comparison as ReadyRetentionHandover;
mod owner;
mod retained;
mod validation;
mod work;

use heph_runtime::VmProvider;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, OwnedReadyRetentionRepository, ResourceProgress, RetainOwnedReady,
};
use std::sync::{Arc, OnceLock};
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
    /// Original error remains retained by the coordinator with any acquired guard.
    #[error("Ready retention remains held: {0}")]
    Held(#[source] Arc<ReadyRetentionError>),
}

/// Nonsecret inspection of one registered retention operation; no IO authority.
#[derive(Debug, Clone)]
pub struct ReadyRetentionPending {
    /// Original volume whose intake is serialized.
    pub volume_id: uuid::Uuid,
    /// Exact originally submitted retention operation.
    pub operation_id: recipe_application::DeploymentAttemptId,
    /// A completed pass returned an error and awaits explicit current retry.
    pub held: bool,
    /// The service still retains its actual non-clone physical guard.
    pub physical_custody: bool,
}
/// One-volume owning coordinator; actual provider and journal family stay retained.
pub struct ReadyRetentionCoordinator {
    actor: Arc<dyn OwnedReadyRetentionRepository>,
    worker: Arc<dyn VolumeReadyRetentionRepository>,
    local: Arc<LocalVolumeStore>,
    provider: Arc<dyn VmProvider>,
    pending: OnceLock<Arc<owner::Registry>>,
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
            pending: OnceLock::new(),
        }
    }
    /// Retains one positive Ready birth under a current manager's real Remove.
    ///
    /// Registers owned work before the first await. Caller cancellation drops
    /// only its reply waiter; unresolved worker/actor errors keep acquired custody
    /// and the exact original proof reachable through this retained coordinator.
    ///
    /// # Errors
    /// Lost management, contradictory history, changed backing or uncertain
    /// completion is held. No automatic rights-loss retry or new probe follows.
    pub async fn retain(
        &self,
        identity: &AuthenticatedIdentity,
        request: &RetainOwnedReady,
        claim: &OwnedProvisioningClaim,
    ) -> Result<ResourceProgress, ReadyRetentionError> {
        Box::pin(owner::submit(self, identity, request, claim, false, None)).await
    }

    /// Retries only an already registered exact request/claim under CURRENT auth.
    ///
    /// Reuses the same acquired guard and observation; never formats/reprobes or
    /// borrows the saved actor identity. Different-command handover is separate.
    ///
    /// # Errors
    /// Missing owner, changed request/claim/scope or current authorization failure
    /// denies the retry and preserves the original held operation.
    pub async fn retry(
        &self,
        identity: &AuthenticatedIdentity,
        request: &RetainOwnedReady,
        claim: &OwnedProvisioningClaim,
    ) -> Result<ResourceProgress, ReadyRetentionError> {
        Box::pin(owner::submit(self, identity, request, claim, true, None)).await
    }

    // Only concrete App removal assembly supplies Source8-qualified comparison
    // data. This is not a public proof/custody/permission injection boundary.
    pub(crate) async fn handover(
        &self,
        identity: &AuthenticatedIdentity,
        request: &RetainOwnedReady,
        claim: &OwnedProvisioningClaim,
        comparison: ReadyRetentionHandover,
    ) -> Result<ResourceProgress, ReadyRetentionError> {
        Box::pin(owner::submit(
            self,
            identity,
            request,
            claim,
            false,
            Some(comparison),
        ))
        .await
    }

    /// Returns registered-operation status without exposing a guard or provider.
    #[must_use]
    pub fn pending_retention(&self) -> Vec<ReadyRetentionPending> {
        self.pending
            .get()
            .map_or_else(Vec::new, |registry| owner::inspect(registry))
    }
}
