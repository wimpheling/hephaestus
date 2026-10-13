//! Explicit original, active fresh-manager and inactive completed-history branches.

mod initial;
pub(super) mod pending;

use super::{Context, Mode, PreparedInstanceDrainError, Request};
use recipe_application::{
    ActiveDeploymentEffectRepository, DeploymentError, DeploymentIntent, EffectClaim,
    ExistingInstanceClosureRepository, ExistingInstanceClosureRequest, OriginalInstanceClosure,
    PlannedResourceIdentity, PreparedInstanceClosureRequest, PreparedInstanceSafetyRepository,
    PreparedInstanceSafetyTarget, ResourceAction, ResourceEffect, ResourceProgress,
};
use std::sync::Arc;

pub(super) struct Original {
    pub request: Request,
    pub intent: Option<DeploymentIntent>,
    pub target: Option<PreparedInstanceSafetyTarget>,
    pub claim: Option<EffectClaim>,
    effect: Option<ResourceEffect>,
    pub closure_request: Option<PreparedInstanceClosureRequest>,
    pub closure: Option<OriginalInstanceClosure>,
    pub resumed_publications: Vec<pending::Publication>,
    pub first_error: Option<Arc<PreparedInstanceDrainError>>,
    pub last_error: Option<Arc<PreparedInstanceDrainError>>,
    pub completions: Vec<super::execution::CompletionOperation>,
    pub completion_errors: Vec<Arc<PreparedInstanceDrainError>>,
}
impl Original {
    pub const fn new(request: Request) -> Self {
        Self {
            request,
            intent: None,
            target: None,
            claim: None,
            effect: None,
            closure_request: None,
            closure: None,
            resumed_publications: Vec::new(),
            first_error: None,
            last_error: None,
            completions: Vec::new(),
            completion_errors: Vec::new(),
        }
    }
}

pub(super) struct Active {
    pub target: PreparedInstanceSafetyTarget,
    pub claim: EffectClaim,
    pub removal: release_domain::InstanceRemovalId,
}
pub(super) enum Admission {
    Active(Box<Active>),
    Completed(ResourceProgress),
}

pub(super) async fn prepare(
    context: &Context,
    request: &Request,
    original: &mut Original,
) -> Result<Admission, PreparedInstanceDrainError> {
    context.controller.validate_owners()?;
    compare_request(request, original)?;
    let (snapshot, endpoint) = match request.mode {
        Mode::Original => (
            context
                .deployment
                .admit_remove(&request.identity, request.remove)
                .await?
                .snapshot,
            pending::Endpoint::Initial,
        ),
        Mode::Resume => {
            let resume = context
                .deployment
                .admit_remove_resume(&request.identity, request.remove)
                .await?;
            // Complete qualified closed history wins without reconstructing
            // Pending or comparing today's deployment CAS to publisher B's first.
            let endpoint = pending::select(
                resume.closures(),
                resume.pending_instance_drains(),
                &request.resource,
            )?;
            (resume.admission().snapshot.clone(), endpoint)
        }
    };
    let plan = snapshot
        .intent
        .resources()
        .get(&request.resource)
        .ok_or(DeploymentError::Unavailable)?;
    plan.validate_action(ResourceAction::Drain)?;
    if !matches!(plan.identity(), PlannedResourceIdentity::Instance { .. }) {
        return Err(DeploymentError::InvalidAction.into());
    }
    if original
        .intent
        .as_ref()
        .is_some_and(|intent| intent != &snapshot.intent)
    {
        return Err(DeploymentError::IntentMismatch.into());
    }
    original.intent = Some(snapshot.intent.clone());
    let target = context
        .safety
        .load_original_target(
            &request.identity,
            request.remove.command,
            &snapshot.intent,
            &request.resource,
        )
        .await?;
    if original.target.as_ref().is_some_and(|old| old != &target) {
        return Err(DeploymentError::IntentMismatch.into());
    }
    original.target = Some(target.clone());
    let active = context
        .preparation
        .load_active_effect(
            &request.identity,
            request.remove.command,
            request.remove.deployment_id,
            &request.resource,
        )
        .await?;
    match endpoint {
        pending::Endpoint::Existing(closure) => {
            existing(
                context,
                request,
                original,
                snapshot.intent,
                target,
                active,
                *closure,
            )
            .await
        }
        pending::Endpoint::Pending(pending) => {
            Box::pin(pending::publish(
                context, request, original, snapshot, target, active, *pending,
            ))
            .await
        }
        pending::Endpoint::Initial => {
            initial::initial(context, request, original, snapshot, target, active).await
        }
    }
}

async fn existing(
    context: &Context,
    request: &Request,
    original: &mut Original,
    intent: DeploymentIntent,
    target: PreparedInstanceSafetyTarget,
    active: Option<EffectClaim>,
    closure: OriginalInstanceClosure,
) -> Result<Admission, PreparedInstanceDrainError> {
    if (matches!(
        &closure.publication_origin,
        recipe_application::InstanceClosurePublicationOrigin::OriginalDrain
    ) && closure.expected_version != request.expected_instance_version)
        || original.closure.as_ref().is_some_and(|old| old != &closure)
    {
        return Err(DeploymentError::IntentMismatch.into());
    }
    existing_qualified(context, request, original, intent, target, active, closure).await
}

// OriginalDrain retains its caller CAS check. A tagged historical continuation
// or positive publication winner still requires the same protected loader below.
async fn existing_qualified(
    context: &Context,
    request: &Request,
    original: &mut Original,
    intent: DeploymentIntent,
    target: PreparedInstanceSafetyTarget,
    active: Option<EffectClaim>,
    closure: OriginalInstanceClosure,
) -> Result<Admission, PreparedInstanceDrainError> {
    if original.closure.as_ref().is_some_and(|old| old != &closure) {
        return Err(DeploymentError::IntentMismatch.into());
    }
    retain_claim(original, &closure.drain, &target)?;
    original.closure = Some(closure.clone());
    let expected = ExistingInstanceClosureRequest::new(
        request.remove.command,
        intent,
        target.clone(),
        closure.clone(),
    )?;
    let loaded = context
        .safety
        .load_existing_instance_closure(&request.identity, &expected)
        .await;
    // Only the ACTUAL protected current-manager loader result permits use of
    // tagged stored CAS. The tag alone is comparison data, never authority.
    pending::compare_loaded(&closure, loaded)?;
    match existing_disposition(active.as_ref(), &closure)? {
        ExistingDisposition::Active => Ok(Admission::Active(Box::new(Active {
            target,
            claim: closure.drain,
            removal: closure.removal_id,
        }))),
        ExistingDisposition::CompletedObservationRequired => context
            .deployment
            .observe_completed_instance_drain(&request.identity, &expected)
            .await
            .map(Admission::Completed)
            .map_err(Into::into),
    }
}

pub(super) fn compare_request(
    request: &Request,
    original: &Original,
) -> Result<(), DeploymentError> {
    if request.remove.deployment_id != original.request.remove.deployment_id
        || request.resource != original.request.resource
        || request.expected_instance_version != original.request.expected_instance_version
        || (request.mode == Mode::Original && request.remove != original.request.remove)
    {
        return Err(DeploymentError::IntentMismatch);
    }
    request.remove.command.validate(
        &request.identity,
        recipe_application::DeploymentOperation::Remove,
    )
}

fn retain_claim(
    original: &mut Original,
    claim: &EffectClaim,
    target: &PreparedInstanceSafetyTarget,
) -> Result<(), DeploymentError> {
    let create = target.original();
    if claim.action != ResourceAction::Drain
        || claim.deployment_id != create.deployment_id
        || claim.resource != create.resource
        || claim.identity != create.identity
        || claim.input_hash != create.input_hash
        || original.claim.as_ref().is_some_and(|old| old != claim)
    {
        return Err(DeploymentError::IntentMismatch);
    }
    original.claim = Some(claim.clone());
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ExistingDisposition {
    Active,
    CompletedObservationRequired,
}

pub(super) fn existing_disposition(
    active: Option<&EffectClaim>,
    closure: &OriginalInstanceClosure,
) -> Result<ExistingDisposition, DeploymentError> {
    match active {
        Some(claim) if claim == &closure.drain => Ok(ExistingDisposition::Active),
        Some(_) => Err(DeploymentError::ReconciliationRequired),
        None => Ok(ExistingDisposition::CompletedObservationRequired),
    }
}
