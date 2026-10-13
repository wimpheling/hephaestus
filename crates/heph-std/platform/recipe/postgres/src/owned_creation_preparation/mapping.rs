use crate::execution_rows::ClaimWire;
use recipe_application::{
    DeploymentError, DeploymentIntent, DeploymentOperation, EffectClaim, PlannedResourceIdentity,
    PreparedOwnedVolumeCreation, ResourceAction, ResourceOwnership,
};
use recipe_domain::ResolvedResource;
use release_domain::ContentHash;
use uuid::Uuid;
use volume_domain::{
    VolumeCreationIdentity, VolumeCreationIdentityWire, VolumeCreationOperationId,
    VolumeOwnershipScopeId,
};

#[cfg(test)]
#[path = "mapping_tests.rs"]
mod tests;

pub fn canonical_claim(claim: &EffectClaim) -> Result<Vec<u8>, DeploymentError> {
    serde_json::to_vec(&ClaimWire::new(claim)?).map_err(|_| DeploymentError::Serialization)
}

pub fn derive(
    intent: &DeploymentIntent,
    claim: &EffectClaim,
) -> Result<PreparedOwnedVolumeCreation, DeploymentError> {
    let plan = intent
        .resources()
        .get(&claim.resource)
        .ok_or(DeploymentError::Unavailable)?;
    let (
        PlannedResourceIdentity::Volume {
            id,
            filesystem_uuid: Some(filesystem_uuid),
        },
        ResolvedResource::Volume(volume),
    ) = (plan.identity(), plan.intent())
    else {
        return Err(DeploymentError::InvalidAction);
    };
    if claim.command.operation() != DeploymentOperation::Install
        || claim.action != ResourceAction::Create
        || plan.ownership() != ResourceOwnership::Owned
        || plan.identity() != claim.identity
        || plan.input_hash() != claim.input_hash
        || claim.deployment_id != intent.id()
    {
        return Err(DeploymentError::InvalidAction);
    }
    let bytes = canonical_claim(claim)?;
    let digest = ContentHash::digest(&bytes);
    let scope = Uuid::new_v5(
        &claim.deployment_id.as_uuid(),
        b"hephaestus-volume-owner-scope-v1\0",
    );
    let mut tagged = b"hephaestus-volume-effect-v1\0".to_vec();
    tagged.extend_from_slice(digest.as_bytes());
    let effect = Uuid::new_v5(&claim.provenance.attempt_id.as_uuid(), &tagged);
    let operation = Uuid::new_v5(&effect, b"hephaestus-volume-register-v1\0");
    let creation = VolumeCreationIdentity::try_from(VolumeCreationIdentityWire {
        scope_id: VolumeOwnershipScopeId::from_uuid(scope)
            .map_err(|_| DeploymentError::InvalidIdentifier)?,
        operation_id: VolumeCreationOperationId::from_uuid(operation)
            .map_err(|_| DeploymentError::InvalidIdentifier)?,
        command_id: claim.command.id().as_uuid(),
        effect_id: effect,
        attempt_id: claim.provenance.attempt_id.as_uuid(),
        actor_id: claim.provenance.actor_id.as_uuid(),
        request_id: claim.provenance.request_id.as_uuid(),
        resource_version: claim.resource_version,
        generation: claim.generation,
        input_hash: *claim.input_hash.as_bytes(),
        intent_hash: *intent.input_hash().as_bytes(),
    })
    .map_err(|_| DeploymentError::InputConflict)?;
    Ok(PreparedOwnedVolumeCreation {
        mapping_version: 1,
        claim_digest: digest,
        creation,
        volume_id: id,
        project_id: intent.project_id().as_uuid(),
        capacity_bytes: volume.capacity_bytes,
        filesystem_uuid,
    })
}
