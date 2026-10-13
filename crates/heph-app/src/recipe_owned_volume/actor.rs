//! Per-effect admission wrapper: every Local `fresh_actor` call rechecks whole intent.

use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    ActiveDeploymentEffectRepository, DeploymentError, DeploymentExecutionPreparationRepository,
    EffectClaim, PreparedOwnedVolumeCreation,
};
use volume_trait::{
    BeginOwnedProvisioning, OwnedBackingObservation, OwnedProvisioningClaim,
    OwnedProvisioningContext, VolumeError, VolumeOwnedProvisioningRepository,
};

use super::{PreparedOwnedVolumeCreator, validation};

pub struct EffectActor {
    pub service: PreparedOwnedVolumeCreator,
    pub claim: EffectClaim,
    pub mapped: PreparedOwnedVolumeCreation,
    pub request: BeginOwnedProvisioning,
}
#[async_trait]
impl VolumeOwnedProvisioningRepository for EffectActor {
    async fn begin_owned_provisioning(
        &self,
        identity: &AuthenticatedIdentity,
        request: &BeginOwnedProvisioning,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        if request != &self.request {
            return Err(VolumeError::IntentConflict);
        }
        fresh(&self.service, identity, &self.claim, &self.mapped)
            .await
            .map_err(|error| admission_error(&error))?;
        self.service
            .actor
            .begin_owned_provisioning(identity, request)
            .await
    }
    async fn owned_reconciliation_context(
        &self,
        _claim: &OwnedProvisioningClaim,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        Err(VolumeError::PermissionDenied) // Actor composition never impersonates a worker.
    }
    async fn record_owned_observation(
        &self,
        _claim: &OwnedProvisioningClaim,
        _observation: &OwnedBackingObservation,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        Err(VolumeError::PermissionDenied)
    }
}
pub async fn fresh(
    service: &PreparedOwnedVolumeCreator,
    identity: &AuthenticatedIdentity,
    claim: &EffectClaim,
    mapped: &PreparedOwnedVolumeCreation,
) -> Result<(), DeploymentError> {
    claim
        .command
        .validate(identity, recipe_application::DeploymentOperation::Install)?;
    let prepared = service
        .preparation
        .prepare_execution(identity, claim.command, claim.deployment_id)
        .await?;
    let active = service
        .preparation
        .load_active_effect(
            identity,
            claim.command,
            claim.deployment_id,
            &claim.resource,
        )
        .await?;
    if active.as_ref() != Some(claim) {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let snapshot = service
        .deployment
        .inspect(identity, claim.deployment_id)
        .await?;
    validation::child(
        &prepared.preparation,
        &snapshot,
        claim.command,
        &claim.resource,
    )?;
    validation::claim(&snapshot, claim.command, claim)?;
    let current = service
        .deployment
        .prepare_owned_volume_creation(identity, claim)
        .await?;
    if &current != mapped {
        return Err(DeploymentError::InputConflict);
    }
    validation::registration(&snapshot, claim, mapped)?;
    Ok(())
}
const fn admission_error(error: &DeploymentError) -> VolumeError {
    match error {
        DeploymentError::AuthorizationDenied => VolumeError::PermissionDenied,
        _ => VolumeError::IntentConflict,
    }
}
