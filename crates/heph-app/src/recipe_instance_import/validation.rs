use capability_domain::CapabilitySlotKey;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentExecutionPreparation, DeploymentLifecycle,
    DeploymentOperation, DeploymentSnapshot, EffectClaim, PlannedResourceIdentity,
    PreparedInstanceImport, ResourceAction, ResourceOwnership,
};

pub fn child<'a>(
    preparation: &'a DeploymentExecutionPreparation,
    snapshot: &DeploymentSnapshot,
    resource: &CapabilitySlotKey,
) -> Result<&'a PreparedInstanceImport, DeploymentError> {
    let plan = snapshot
        .intent
        .resources()
        .get(resource)
        .ok_or(DeploymentError::Unavailable)?;
    let prepared = preparation
        .instances()
        .iter()
        .find(|child| child.resource() == resource)
        .ok_or(DeploymentError::InvalidAction)?;
    let identity = PlannedResourceIdentity::Instance {
        id: prepared.instance_id(),
        revision_id: prepared.revision_id(),
    };
    if preparation.deployment_id() != snapshot.intent.id()
        || preparation.project_id() != snapshot.intent.project_id()
        || preparation.intent_hash() != snapshot.intent.input_hash()
        || plan.ownership() != ResourceOwnership::Owned
        || plan.identity() != identity
        || prepared.project_id() != snapshot.intent.project_id()
        || !matches!(
            snapshot.lifecycle,
            DeploymentLifecycle::Installing
                | DeploymentLifecycle::Installed
                | DeploymentLifecycle::RecoveryRequired
        )
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(prepared)
}

pub fn claim(
    original: CommandIdentity,
    snapshot: &DeploymentSnapshot,
    prepared: &PreparedInstanceImport,
    claim: &EffectClaim,
) -> Result<(), DeploymentError> {
    let plan = snapshot
        .intent
        .resources()
        .get(prepared.resource())
        .ok_or(DeploymentError::Unavailable)?;
    if original.operation() != DeploymentOperation::Install
        || claim.command != original
        || claim.deployment_id != snapshot.intent.id()
        || claim.resource != *prepared.resource()
        || claim.identity != plan.identity()
        || claim.input_hash != plan.input_hash()
        || claim.action != ResourceAction::Create
        || claim.provenance.actor_id != original.actor_id()
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
