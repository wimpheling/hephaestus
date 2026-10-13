//! Proposal retention precedes actual protected fenced claim publication.
use super::{Context, PreparedInstanceTerminalError, Request, retained::Original};
use recipe_application::{
    AttemptProvenance, DeploymentAttemptId, DeploymentError, DeploymentSnapshot, EffectClaim,
    ResourceAction, ResourceEffect,
};
use uuid::Uuid;

pub(super) async fn claim(
    context: &Context,
    request: &Request,
    original: &mut Original,
    snapshot: &DeploymentSnapshot,
    action: ResourceAction,
) -> Result<EffectClaim, PreparedInstanceTerminalError> {
    let progress = snapshot
        .resources
        .get(&request.resource)
        .ok_or(DeploymentError::Unavailable)?;
    let proposal = proposal(
        request,
        original,
        snapshot.version,
        progress.version,
        action,
    )?;
    // The retained full current-authenticated proposal/attempt precedes DB commit.
    let claim = context
        .deployment
        .claim_resource_effect(&request.identity, proposal.clone())
        .await?;
    compare(&proposal, &claim)?;
    original.claim(&claim)?;
    Ok(claim)
}
pub(super) fn proposal(
    request: &Request,
    original: &mut Original,
    deployment_version: u64,
    resource_version: u64,
    action: ResourceAction,
) -> Result<ResourceEffect, DeploymentError> {
    let current = AttemptProvenance::new(
        &request.identity,
        DeploymentAttemptId::from_uuid(Uuid::new_v4())?,
    )?;
    // Same current actor/request/logical command and exact before versions reuse
    // the original proposal. Changed genuine admission appends, never erases it.
    if let Some(previous) = original.proposals.iter().find(|old| {
        old.command == request.remove.command
            && old.deployment_id == request.remove.deployment_id
            && old.resource == request.resource
            && old.action == action
            && old.expected_deployment_version == deployment_version
            && old.expected_resource_version == resource_version
            && old.provenance.actor_id == current.actor_id
            && old.provenance.request_id == current.request_id
    }) {
        return Ok(previous.clone());
    }
    let effect = ResourceEffect {
        command: request.remove.command,
        deployment_id: request.remove.deployment_id,
        resource: request.resource.clone(),
        expected_deployment_version: deployment_version,
        expected_resource_version: resource_version,
        action,
        provenance: current,
    };
    original.proposals.push(effect.clone());
    Ok(effect)
}
pub(super) fn compare(
    proposal: &ResourceEffect,
    claim: &EffectClaim,
) -> Result<(), DeploymentError> {
    if claim.command != proposal.command
        || claim.deployment_id != proposal.deployment_id
        || claim.resource != proposal.resource
        || claim.action != proposal.action
        || claim.provenance != proposal.provenance
        || Some(claim.resource_version) != proposal.expected_resource_version.checked_add(1)
    {
        return Err(DeploymentError::InputConflict);
    }
    Ok(())
}

// Positive current-authorized recovery may restore a lost reply, but cannot
// substitute a different attempt after this owner already published a proposal.
pub(super) fn restore_loaded(
    original: &mut Original,
    claim: &EffectClaim,
) -> Result<(), DeploymentError> {
    let mut proposed = original
        .proposals
        .iter()
        .filter(|old| old.action == claim.action);
    if proposed.clone().next().is_some() && !proposed.any(|old| compare(old, claim).is_ok()) {
        return Err(DeploymentError::InputConflict);
    }
    original.claim(claim)
}
