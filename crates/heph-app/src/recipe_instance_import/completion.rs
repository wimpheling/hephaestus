use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, DeploymentAttemptId, DeploymentError, DiagnosticCode, EffectClaim,
    EffectEvidence, EffectOutcome, InstallProgress, ReconciledOutcome, ResourceProgress,
};
use recipe_postgres::VerifiedOutcome;
use uuid::Uuid;

use super::{InstanceImportError, InstanceImportFailure, PreparedInstanceImporter};

pub async fn finish(
    service: &PreparedInstanceImporter,
    identity: &AuthenticatedIdentity,
    claim: &EffectClaim,
) -> Result<ResourceProgress, DeploymentError> {
    let snapshot = service
        .deployment
        .inspect(identity, claim.deployment_id)
        .await?;
    let progress = snapshot
        .resources
        .get(&claim.resource)
        .ok_or(DeploymentError::Unavailable)?;
    if progress.active_attempt.is_none() && progress.install == InstallProgress::Ready {
        return Ok(progress.clone()); // Another exact completion won this same claim.
    }
    if progress.active_attempt != Some(claim.provenance.attempt_id) {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let evidence = EffectEvidence {
        identity: claim.identity,
        input_hash: claim.input_hash,
        outcome: EffectOutcome::Applied,
    };
    if progress.version == claim.resource_version {
        service
            .recorder
            .record(claim, claim.provenance.attempt_id, VerifiedOutcome::Applied)
            .await?;
        return service
            .deployment
            .complete_resource_effect(identity, claim, evidence)
            .await;
    }
    // Ambiguity changes current progress, never the original claim version.
    let provenance =
        AttemptProvenance::new(identity, DeploymentAttemptId::from_uuid(Uuid::new_v4())?)?;
    service
        .recorder
        .record(claim, provenance.attempt_id, VerifiedOutcome::Applied)
        .await?;
    service
        .deployment
        .record_reconciliation(
            identity,
            claim.command,
            claim,
            provenance,
            ReconciledOutcome::Applied(evidence),
        )
        .await
}

pub async fn hold(
    service: &PreparedInstanceImporter,
    identity: &AuthenticatedIdentity,
    claim: &EffectClaim,
    failure: InstanceImportFailure,
) -> InstanceImportError {
    let ambiguity_error = service
        .deployment
        .mark_resource_ambiguous(identity, claim, DiagnosticCode::ProviderOutcomeUnknown)
        .await
        .err();
    InstanceImportError::Held {
        failure,
        ambiguity_error,
    }
}
