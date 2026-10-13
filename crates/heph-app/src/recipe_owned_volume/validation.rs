use capability_domain::CapabilitySlotKey;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentExecutionPreparation, DeploymentLifecycle,
    DeploymentOperation, DeploymentSnapshot, EffectClaim, PlannedResourceIdentity,
    PreparedOwnedVolumeCreation, ResourceAction, ResourceOwnership,
};
use recipe_domain::ResolvedResource;
use uuid::Uuid;
use volume_trait::{OwnedVolumeRegistration, VolumeRegistration};

pub fn child(
    prepared: &DeploymentExecutionPreparation,
    snapshot: &DeploymentSnapshot,
    command: CommandIdentity,
    resource: &CapabilitySlotKey,
) -> Result<(), DeploymentError> {
    let plan = snapshot
        .intent
        .resources()
        .get(resource)
        .ok_or(DeploymentError::Unavailable)?;
    let (
        PlannedResourceIdentity::Volume {
            filesystem_uuid: Some(_),
            ..
        },
        ResolvedResource::Volume(volume),
    ) = (plan.identity(), plan.intent())
    else {
        return Err(DeploymentError::InvalidAction);
    };
    if prepared.command() != command
        || command.operation() != DeploymentOperation::Install
        || prepared.deployment_id() != snapshot.intent.id()
        || prepared.project_id() != snapshot.intent.project_id()
        || prepared.intent_hash() != snapshot.intent.input_hash()
        || plan.ownership() != ResourceOwnership::Owned
        || volume.external_id.is_some()
        || !matches!(
            snapshot.lifecycle,
            DeploymentLifecycle::Installing
                | DeploymentLifecycle::Installed
                | DeploymentLifecycle::RecoveryRequired
        )
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}
pub fn claim(
    snapshot: &DeploymentSnapshot,
    command: CommandIdentity,
    claim: &EffectClaim,
) -> Result<(), DeploymentError> {
    let plan = snapshot
        .intent
        .resources()
        .get(&claim.resource)
        .ok_or(DeploymentError::Unavailable)?;
    if claim.command != command
        || claim.deployment_id != snapshot.intent.id()
        || claim.identity != plan.identity()
        || claim.input_hash != plan.input_hash()
        || claim.action != ResourceAction::Create
        || claim.provenance.actor_id != command.actor_id()
        || snapshot
            .resources
            .get(&claim.resource)
            .and_then(|state| state.active_attempt)
            != Some(claim.provenance.attempt_id)
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}
pub fn registration(
    snapshot: &DeploymentSnapshot,
    claim: &EffectClaim,
    mapped: &PreparedOwnedVolumeCreation,
) -> Result<OwnedVolumeRegistration, DeploymentError> {
    let plan = snapshot
        .intent
        .resources()
        .get(&claim.resource)
        .ok_or(DeploymentError::Unavailable)?;
    let (
        PlannedResourceIdentity::Volume {
            id,
            filesystem_uuid: Some(filesystem_uuid),
        },
        ResolvedResource::Volume(volume),
    ) = (claim.identity, plan.intent())
    else {
        return Err(DeploymentError::InvalidAction);
    };
    let fields = mapped.creation.fields();
    let mut tagged = b"hephaestus-volume-effect-v1\0".to_vec();
    tagged.extend_from_slice(mapped.claim_digest.as_bytes());
    let effect = Uuid::new_v5(&claim.provenance.attempt_id.as_uuid(), &tagged);
    if mapped.mapping_version != 1
        || mapped.volume_id != id
        || mapped.filesystem_uuid != filesystem_uuid
        || mapped.project_id != snapshot.intent.project_id().as_uuid()
        || mapped.capacity_bytes != volume.capacity_bytes
        || fields.command_id != claim.command.id().as_uuid()
        || fields.attempt_id != claim.provenance.attempt_id.as_uuid()
        || fields.actor_id != claim.provenance.actor_id.as_uuid()
        || fields.request_id != claim.provenance.request_id.as_uuid()
        || fields.resource_version != claim.resource_version
        || fields.generation != claim.generation
        || fields.input_hash != *claim.input_hash.as_bytes()
        || fields.intent_hash != *snapshot.intent.input_hash().as_bytes()
        || fields.scope_id.as_uuid()
            != Uuid::new_v5(
                &claim.deployment_id.as_uuid(),
                b"hephaestus-volume-owner-scope-v1\0",
            )
        || fields.effect_id != effect
        || fields.operation_id.as_uuid()
            != Uuid::new_v5(&effect, b"hephaestus-volume-register-v1\0")
    {
        return Err(DeploymentError::InputConflict);
    }
    Ok(OwnedVolumeRegistration::new(
        VolumeRegistration::new(
            id,
            mapped.project_id,
            mapped.capacity_bytes,
            filesystem_uuid,
        )
        .map_err(|_| DeploymentError::InputConflict)?,
        mapped.creation.clone(),
    ))
}
