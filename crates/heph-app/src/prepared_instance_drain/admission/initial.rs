//! Original publication path, unchanged actor/claim/closure behavior.
use super::super::{Context, Mode, PreparedInstanceDrainError, Request};
use super::{Active, Admission, Original, retain_claim};
use recipe_application::{
    AttemptProvenance, DeploymentAttemptId, DeploymentError, EffectClaim,
    PreparedInstanceClosureRepository, PreparedInstanceClosureRequest,
    PreparedInstanceSafetyTarget, ResourceAction, ResourceEffect,
};
use uuid::Uuid;

pub(super) async fn initial(
    context: &Context,
    request: &Request,
    original: &mut Original,
    snapshot: recipe_application::DeploymentSnapshot,
    target: PreparedInstanceSafetyTarget,
    active: Option<EffectClaim>,
) -> Result<Admission, PreparedInstanceDrainError> {
    // An active historical closure must never be replaced by a new claim/closure.
    if request.mode == Mode::Resume && (active.is_some() || original.closure.is_some()) {
        return Err(DeploymentError::ReconciliationRequired.into());
    }
    let progress = snapshot
        .resources
        .get(&request.resource)
        .ok_or(DeploymentError::Unavailable)?;
    if active.is_none() && progress.removal != recipe_application::RemovalProgress::Pending {
        return Err(DeploymentError::ReconciliationRequired.into());
    }
    let claim = if let Some(claim) = active {
        claim
    } else {
        if original.effect.is_none() {
            original.effect = Some(ResourceEffect {
                command: request.remove.command,
                deployment_id: request.remove.deployment_id,
                resource: request.resource.clone(),
                expected_deployment_version: snapshot.version,
                expected_resource_version: progress.version,
                action: ResourceAction::Drain,
                provenance: AttemptProvenance::new(
                    &request.identity,
                    DeploymentAttemptId::from_uuid(Uuid::new_v4())?,
                )?,
            });
        }
        let effect = original
            .effect
            .as_ref()
            .ok_or(DeploymentError::ReconciliationRequired)?;
        context
            .deployment
            .claim_resource_effect(&request.identity, effect.clone())
            .await?
    };
    retain_claim(original, &claim, &target)?;
    if claim.command != request.remove.command {
        return Err(DeploymentError::InputConflict.into());
    }
    let closure_request = PreparedInstanceClosureRequest::new(
        snapshot.intent.clone(),
        target.clone(),
        claim.clone(),
        request.expected_instance_version,
    )?;
    if original
        .closure_request
        .as_ref()
        .is_some_and(|old| old != &closure_request)
    {
        return Err(DeploymentError::IntentMismatch.into());
    }
    // Retain original claim and complete closure request BEFORE actor commit await.
    original.closure_request = Some(closure_request.clone());
    let closed = context
        .safety
        .close_for_remove(&request.identity, &closure_request)
        .await?;
    if closed.target != target || closed.removal_id != closure_request.removal_id() {
        return Err(DeploymentError::IntentMismatch.into());
    }
    Ok(Admission::Active(Box::new(Active {
        target,
        claim,
        removal: closed.removal_id,
    })))
}
