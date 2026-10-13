use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    ActiveDeploymentEffectRepository, AttemptProvenance, CommandIdentity, DeploymentAttemptId,
    DeploymentError, DeploymentExecutionPreparationRepository, DeploymentId, DeploymentLifecycle,
    DeploymentOperation, EffectClaim, InstallProgress, ResourceAction, ResourceEffect,
    ResourceProgress,
};
use uuid::Uuid;

use super::{
    InstanceImportError, InstanceImportFailure, PreparedInstanceImporter, completion,
    to_import_command, validation,
};

pub async fn import(
    service: &PreparedInstanceImporter,
    identity: &AuthenticatedIdentity,
    command: CommandIdentity,
    deployment: DeploymentId,
    resource: &CapabilitySlotKey,
) -> Result<ResourceProgress, InstanceImportError> {
    command.validate(identity, DeploymentOperation::Install)?;
    let prepared = service
        .preparation
        .prepare_execution(identity, command, deployment)
        .await?;
    let snapshot = service.deployment.inspect(identity, deployment).await?;
    let child = validation::child(&prepared.preparation, &snapshot, resource)?;
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
        // Exact113 input/current authority plus protected106 completed state is
        // known completion. No new import/claim/event is needed merely to replay.
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
    validation::claim(
        command,
        &snapshot_after_claim(service, identity, deployment).await?,
        child,
        &claim,
    )?;
    invoke(service, identity, command, &claim).await
}

async fn snapshot_after_claim(
    service: &PreparedInstanceImporter,
    identity: &AuthenticatedIdentity,
    deployment: DeploymentId,
) -> Result<recipe_application::DeploymentSnapshot, DeploymentError> {
    service.deployment.inspect(identity, deployment).await
}

async fn invoke(
    service: &PreparedInstanceImporter,
    identity: &AuthenticatedIdentity,
    command: CommandIdentity,
    claim: &EffectClaim,
) -> Result<ResourceProgress, InstanceImportError> {
    // Recheck after claim commit: source/status/config or a permanent Remove may
    // have changed while awaiting the CAS. This is not a reusable grant stamp.
    let prepared = service
        .preparation
        .prepare_execution(identity, command, claim.deployment_id)
        .await?;
    let active = service
        .preparation
        .load_active_effect(identity, command, claim.deployment_id, &claim.resource)
        .await?;
    if active.as_ref() != Some(claim) {
        return Err(DeploymentError::ReconciliationRequired.into());
    }
    let snapshot = service
        .deployment
        .inspect(identity, claim.deployment_id)
        .await?;
    let child = validation::child(&prepared.preparation, &snapshot, &claim.resource)?;
    validation::claim(command, &snapshot, child, claim)?;
    let input = to_import_command(child);
    // The actual successful SQL command and checked returned instance are the
    // positive observation; matching DTOs/UUIDs alone are never such evidence.
    let result = service
        .release
        .import_agent_with_volumes(identity, input)
        .await;
    match result {
        Ok(id) if id.as_uuid() == child.instance_id().as_uuid() => {
            match completion::finish(service, identity, claim).await {
                Ok(progress) => Ok(progress),
                Err(error) => Err(completion::hold(
                    service,
                    identity,
                    claim,
                    InstanceImportFailure::Completion(error),
                )
                .await),
            }
        }
        Ok(_) => Err(completion::hold(
            service,
            identity,
            claim,
            InstanceImportFailure::IdentityConflict,
        )
        .await),
        Err(error) => Err(completion::hold(
            service,
            identity,
            claim,
            InstanceImportFailure::Release(Box::new(error)),
        )
        .await),
    }
}
