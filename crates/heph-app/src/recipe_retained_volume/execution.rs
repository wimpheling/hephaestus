use identity_domain::AuthenticatedIdentity;
use recipe_application::{DeploymentOperation, EffectClaim, ResourceProgress};
use volume_trait::{
    OwnedProvisioningDiscovery, OwnedProvisioningExpectation, VolumeError,
    VolumeOwnedProvisioningRepository,
};

use super::{PreparedRetainedVolumeVerifier, RetainedVolumeVerificationError, pending, validation};

pub async fn verify(
    service: &PreparedRetainedVolumeVerifier,
    identity: &AuthenticatedIdentity,
    claim: &EffectClaim,
) -> Result<ResourceProgress, RetainedVolumeVerificationError> {
    claim
        .command
        .validate(identity, DeploymentOperation::Install)?;
    if let Some(result) = pending::retry(service, identity, claim).await {
        return result;
    }
    let plan = service
        .actor
        .prepare_retained_external_verification(identity, claim)
        .await?;
    validation::plan(service.provider.as_ref(), &plan, claim)?;
    if let Some(progress) = plan.completed {
        return Ok(progress); // Paired replay never probes the reopened backing.
    }
    let expected = OwnedProvisioningExpectation::new(
        validation::registration(&plan)?,
        service.owner.host().to_owned(),
        service.owner.root().to_path_buf(),
        service.owner.namespace_id(),
    )?;
    let OwnedProvisioningDiscovery::Original(original) = service
        .metadata
        .discover_owned_provisioning(&expected)
        .await?
    else {
        return Err(VolumeError::IntentConflict.into());
    };
    let context = service
        .worker
        .retained_reuse_context(
            &original.claim,
            plan.closure_id,
            plan.retention_operation_id.as_uuid(),
        )
        .await?;
    validation::context(&plan, &expected, &context)?;
    if context.retained.original != *original {
        return Err(RetainedVolumeVerificationError::Correlation);
    }
    let custody = service
        .local
        .observe_retained_owned_reuse(service.worker.as_ref(), &context)
        .await?;
    // No await follows custody acquisition until this actual guard and all its
    // service/provider owners have moved into the detached completion task.
    let completed = pending::start(service.clone(), identity.clone(), plan, custody)?;
    completed
        .await
        .map_err(|_| RetainedVolumeVerificationError::CompletionUnknown)?
}
