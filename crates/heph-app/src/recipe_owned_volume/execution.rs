use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    ActiveDeploymentEffectRepository, AttemptProvenance, CommandIdentity, DeploymentAttemptId,
    DeploymentError, DeploymentExecutionPreparationRepository, DeploymentId, DeploymentLifecycle,
    DeploymentOperation, EffectClaim, InstallProgress, PreparedOwnedVolumeCreation, ResourceAction,
    ResourceEffect, ResourceProgress,
};
use std::sync::Arc;
use uuid::Uuid;
use volume_local::{OwnedReadyCustody, OwnedVolumeMetadata};
use volume_trait::{
    BeginOwnedProvisioning, OwnedBackingPhase, OwnedProvisioningDiscovery,
    OwnedProvisioningExpectation, VolumeError, VolumeOwnedProvisioningRepository,
    VolumeOwnedRegistrationRepository,
};

use super::{
    OwnedVolumeCreateError, OwnedVolumeCreateFailure, PreparedOwnedVolumeCreator, actor,
    completion, validation,
};

pub async fn create(
    service: &PreparedOwnedVolumeCreator,
    identity: &AuthenticatedIdentity,
    command: CommandIdentity,
    deployment: DeploymentId,
    resource: &CapabilitySlotKey,
) -> Result<ResourceProgress, OwnedVolumeCreateError> {
    command.validate(identity, DeploymentOperation::Install)?;
    let prepared = service
        .preparation
        .prepare_execution(identity, command, deployment)
        .await?;
    let snapshot = service.deployment.inspect(identity, deployment).await?;
    validation::child(&prepared.preparation, &snapshot, command, resource)?;
    let active = service
        .preparation
        .load_active_effect(identity, command, deployment, resource)
        .await?;
    let progress = snapshot
        .resources
        .get(resource)
        .ok_or(DeploymentError::Unavailable)?;
    if active.is_none() && progress.install == InstallProgress::Ready {
        if !matches!(
            snapshot.lifecycle,
            DeploymentLifecycle::Installing | DeploymentLifecycle::Installed
        ) {
            return Err(DeploymentError::ReconciliationRequired.into());
        }
        // Protected completion opens runtime admission: no probe/claim/format replay.
        return Ok(progress.clone());
    }
    let claim = if let Some(claim) = active {
        claim
    } else {
        service
            .deployment
            .claim_resource_effect(
                identity,
                ResourceEffect {
                    command,
                    deployment_id: deployment,
                    resource: resource.clone(),
                    expected_deployment_version: snapshot.version,
                    expected_resource_version: progress.version,
                    action: ResourceAction::Create,
                    provenance: AttemptProvenance::new(
                        identity,
                        DeploymentAttemptId::from_uuid(Uuid::new_v4())?,
                    )?,
                },
            )
            .await?
    };
    let snapshot = service.deployment.inspect(identity, deployment).await?;
    validation::claim(&snapshot, command, &claim)?;
    let mapped = service
        .deployment
        .prepare_owned_volume_creation(identity, &claim)
        .await?;
    let result = provision(service, identity, &claim, &mapped).await;
    let custody = match result {
        Ok(custody) => custody,
        Err(failure) => return Err(completion::hold(service, identity, &claim, failure).await),
    };
    // Native custody moves into the protected completion owner BEFORE the caller
    // can cancel an in-flight actor transaction and accidentally release its lock.
    let service = service.clone();
    let identity = identity.clone();
    tokio::spawn(
        async move { completion::commit_held(service, identity, claim, mapped, custody).await },
    )
    .await
    .map_err(|_| OwnedVolumeCreateError::CompletionUnknown)?
}
async fn provision(
    service: &PreparedOwnedVolumeCreator,
    identity: &AuthenticatedIdentity,
    claim: &EffectClaim,
    mapped: &PreparedOwnedVolumeCreation,
) -> Result<OwnedReadyCustody, OwnedVolumeCreateFailure> {
    actor::fresh(service, identity, claim, mapped).await?;
    let snapshot = service
        .deployment
        .inspect(identity, claim.deployment_id)
        .await?;
    let registration = validation::registration(&snapshot, claim, mapped)?;
    let receipt = service
        .actor
        .register_owned(identity, &registration)
        .await?;
    if receipt.intent != registration {
        return Err(VolumeError::IntentConflict.into());
    }
    let expected = OwnedProvisioningExpectation::new(
        registration,
        service.owner.host().to_owned(),
        service.owner.root().to_path_buf(),
        service.owner.namespace_id(),
    )?;
    let discovered = service
        .worker
        .discover_owned_provisioning(&expected)
        .await?;
    let request = BeginOwnedProvisioning {
        purpose: expected.purpose(receipt)?,
        operation_id: expected.first_operation_id(),
        expected_generation: 0,
    };
    let local = service
        .local
        .clone()
        .with_owned_metadata(OwnedVolumeMetadata::new(
            Arc::new(actor::EffectActor {
                service: service.clone(),
                claim: claim.clone(),
                mapped: mapped.clone(),
                request: request.clone(),
            }),
            service.worker.clone(),
            service.owner.clone(),
        ))?;
    let current = match discovered {
        OwnedProvisioningDiscovery::Unadmitted(receipt) => {
            if receipt.intent != *expected.intent() {
                return Err(VolumeError::IntentConflict.into());
            }
            local.provision_owned(identity, &request).await?
        }
        OwnedProvisioningDiscovery::Original(context) => {
            if context.claim.purpose != request.purpose
                || context.claim.operation_id != request.operation_id
                || context.claim.generation != 1
            {
                return Err(VolumeError::IntentConflict.into());
            }
            match context
                .observation
                .as_ref()
                .map(|proof| proof.fields().phase)
            {
                None | Some(OwnedBackingPhase::Recorded) => {
                    // Same operation/fence and fresh actor admission only. The
                    // actual journal refuses unrecorded IO and routes a durable
                    // MayHaveStarted to readonly proof without another mkfs.
                    local.provision_owned(identity, &request).await?
                }
                Some(OwnedBackingPhase::FailedBeforeFormat) => {
                    return Err(VolumeError::ProvisioningUncertain(
                        "original first operation is terminal; no new CAS",
                    )
                    .into());
                }
                _ => local.reconcile_owned(&context.claim).await?,
            }
        }
    };
    actor::fresh(service, identity, claim, mapped).await?;
    Ok(local.verify_owned_ready(&current.claim).await?)
}
