//! Concrete partial-birth custody through worker recording and atomic actor commit.
//!
//! This is backend composition, not a public Remove transport or a provider engine.

use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, OwnedPartialRetentionRepository, PreparedOwnedVolumeCreation,
    ResourceProgress, RetainPartialCreation,
};
use std::sync::Arc;
use volume_local::{LocalVolumeStore, OwnedPartialBirthCustody, PartialBirthCustodyError};
use volume_trait::{OwnedProvisioningClaim, VolumeError, VolumePartialRetentionRepository};

/// Backend result including a commit followed by failed physical validation.
#[derive(Debug, thiserror::Error)]
pub enum PartialRetentionError {
    /// Live command or atomic repository validation failed.
    #[error(transparent)]
    Deployment(#[from] DeploymentError),
    /// Physical custody could not be established or stayed contradictory.
    #[error(transparent)]
    Physical(#[from] PartialBirthCustodyError),
    /// Worker receipt validation failed before actor commit.
    #[error(transparent)]
    Volume(#[from] VolumeError),
    /// Supplied physical claim differs from authoritative durable preparation.
    #[error("owned birth differs from original recipe preparation")]
    Correlation,
    /// The protected completion task ended without a known transaction result.
    #[error("partial retention completion is unknown; inspect with fresh authorization")]
    CompletionUnknown,
    /// The permanent fence committed, then physical validation failed.
    ///
    /// This must never be reported as a rolled-back command. Fresh authorized
    /// retry discovers the committed result without reviving creation.
    #[error("partial retention committed; physical custody needs inspection")]
    CommittedCustodyChanged {
        /// Exact already committed progress, without physical/provider handles.
        progress: ResourceProgress,
    },
}

/// Owning backend service retaining a concrete non-clone Local guard.
pub struct PartialRetentionCoordinator {
    actor: Arc<dyn OwnedPartialRetentionRepository>,
    worker: Arc<dyn VolumePartialRetentionRepository>,
    local: Arc<LocalVolumeStore>,
}
impl PartialRetentionCoordinator {
    /// Injects separate actor/worker roles and the actual canonical Local store.
    #[must_use]
    pub fn new(
        actor: Arc<dyn OwnedPartialRetentionRepository>,
        worker: Arc<dyn VolumePartialRetentionRepository>,
        local: Arc<LocalVolumeStore>,
    ) -> Self {
        Self {
            actor,
            worker,
            local,
        }
    }

    /// Retains an unusable positive partial birth under current admitted removal.
    ///
    /// No worker receipt is accepted in the Remove request. Fresh authorization
    /// precedes discovery/replay. Actual custody remains owned until the awaited
    /// worker receipt and actor transaction both finish, including cancellation.
    ///
    /// # Errors
    /// Rejects denied cleanup, unknown/Ready/foreign birth, active physical holder,
    /// stale observation/CAS or atomic SQL denial. A postcommit validation failure
    /// explicitly carries the committed result rather than claiming rollback.
    pub async fn retain(
        &self,
        identity: &AuthenticatedIdentity,
        request: &RetainPartialCreation,
        claim: &OwnedProvisioningClaim,
    ) -> Result<ResourceProgress, PartialRetentionError> {
        let plan = self
            .actor
            .partial_creation_retention_plan(identity, request)
            .await?;
        compare(&plan.preparation, claim)?;
        if let Some(progress) = plan.completed {
            return Ok(progress);
        }
        let custody = self
            .local
            .observe_owned_partial(self.worker.as_ref(), claim)
            .await?;
        // Once native custody exists, caller cancellation must not drop it while
        // the worker/actor transaction is awaiting completion. This protected completion
        // owns the original verified request; it creates no background identity.
        let actor = self.actor.clone();
        let worker = self.worker.clone();
        let identity = identity.clone();
        let request = request.clone();
        tokio::spawn(async move { commit_held(actor, worker, identity, request, custody).await })
            .await
            .map_err(|_| PartialRetentionError::CompletionUnknown)?
    }
}

async fn commit_held(
    actor: Arc<dyn OwnedPartialRetentionRepository>,
    worker: Arc<dyn VolumePartialRetentionRepository>,
    identity: AuthenticatedIdentity,
    request: RetainPartialCreation,
    mut custody: OwnedPartialBirthCustody,
) -> Result<ResourceProgress, PartialRetentionError> {
    custody.revalidate()?;
    let receipt = worker
        .record_partial_retention_observation(custody.context(), custody.observation())
        .await?;
    custody.revalidate()?;
    let progress = actor
        .retain_partial_creation(&identity, &request, receipt.id.as_uuid())
        .await?;
    if custody.revalidate().is_err() {
        return Err(PartialRetentionError::CommittedCustodyChanged { progress });
    }
    // Explicitly release only after the actor commit (or unwind on failure).
    drop(custody);
    Ok(progress)
}

fn compare(
    prepared: &PreparedOwnedVolumeCreation,
    claim: &OwnedProvisioningClaim,
) -> Result<(), PartialRetentionError> {
    let receipt = claim.purpose.receipt();
    let resource = receipt.intent.registration();
    if prepared.mapping_version != 1
        || prepared.creation != *receipt.intent.creation()
        || prepared.volume_id != resource.id()
        || prepared.project_id != resource.project_id()
        || prepared.capacity_bytes != resource.capacity_bytes()
        || prepared.filesystem_uuid != resource.filesystem_uuid()
    {
        return Err(PartialRetentionError::Correlation);
    }
    Ok(())
}
