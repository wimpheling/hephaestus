use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, DeploymentAttemptId, DeploymentError, DiagnosticCode, EffectClaim,
    EffectEvidence, EffectOutcome, InstallProgress, PreparedOwnedVolumeCreation, ReconciledOutcome,
    ResourceProgress,
};
use recipe_postgres::VerifiedOutcome;
use uuid::Uuid;
use volume_local::OwnedReadyCustody;
use volume_trait::VolumeOwnedProvisioningRepository;

use super::{OwnedVolumeCreateError, OwnedVolumeCreateFailure, PreparedOwnedVolumeCreator, actor};

pub async fn commit_held(
    service: PreparedOwnedVolumeCreator,
    identity: AuthenticatedIdentity,
    claim: EffectClaim,
    mapped: PreparedOwnedVolumeCreation,
    mut custody: OwnedReadyCustody,
) -> Result<ResourceProgress, OwnedVolumeCreateError> {
    let result = finish(&service, &identity, &claim, &mapped, &mut custody).await;
    // Keep the lock while marking ambiguity too; never replace primary failure.
    let result = match result {
        Ok(progress) => Ok(progress),
        Err(failure) => Err(hold(&service, &identity, &claim, failure).await),
    };
    drop(custody); // All awaited recording/commit work has resolved in this owner.
    result
}
async fn finish(
    service: &PreparedOwnedVolumeCreator,
    identity: &AuthenticatedIdentity,
    claim: &EffectClaim,
    mapped: &PreparedOwnedVolumeCreation,
    custody: &mut OwnedReadyCustody,
) -> Result<ResourceProgress, OwnedVolumeCreateFailure> {
    custody.revalidate()?;
    actor::fresh(service, identity, claim, mapped).await?;
    service
        .worker
        .assert_owned_first_ready_verification(&custody.context().claim)
        .await?;
    let snapshot = service
        .deployment
        .inspect(identity, claim.deployment_id)
        .await?;
    let progress = snapshot
        .resources
        .get(&claim.resource)
        .ok_or(DeploymentError::Unavailable)?;
    if progress.active_attempt.is_none() && progress.install == InstallProgress::Ready {
        // This path cannot follow a successful unfinished verification under the
        // same actual flock; treat a contradictory writer as held, never fsck again.
        return Err(DeploymentError::ReconciliationRequired.into());
    }
    if progress.active_attempt != Some(claim.provenance.attempt_id) {
        return Err(DeploymentError::ReconciliationRequired.into());
    }
    let evidence = EffectEvidence {
        identity: claim.identity,
        input_hash: claim.input_hash,
        outcome: EffectOutcome::Applied,
    };
    let progress = if progress.version == claim.resource_version {
        service
            .recorder
            .record(claim, claim.provenance.attempt_id, VerifiedOutcome::Applied)
            .await?;
        custody.revalidate()?;
        actor::fresh(service, identity, claim, mapped).await?;
        service
            .deployment
            .complete_resource_effect(identity, claim, evidence)
            .await?
    } else {
        let provenance =
            AttemptProvenance::new(identity, DeploymentAttemptId::from_uuid(Uuid::new_v4())?)?;
        service
            .recorder
            .record(claim, provenance.attempt_id, VerifiedOutcome::Applied)
            .await?;
        custody.revalidate()?;
        actor::fresh(service, identity, claim, mapped).await?;
        service
            .deployment
            .record_reconciliation(
                identity,
                claim.command,
                claim,
                provenance,
                ReconciledOutcome::Applied(evidence),
            )
            .await?
    };
    // Do not fsck/recheck clean flags after commit: runtime admission may now open.
    Ok(progress)
}
pub async fn hold(
    service: &PreparedOwnedVolumeCreator,
    identity: &AuthenticatedIdentity,
    claim: &EffectClaim,
    failure: OwnedVolumeCreateFailure,
) -> OwnedVolumeCreateError {
    let ambiguity_error = service
        .deployment
        .mark_resource_ambiguous(identity, claim, DiagnosticCode::ProviderOutcomeUnknown)
        .await
        .err();
    OwnedVolumeCreateError::Held {
        failure,
        ambiguity_error,
    }
}
