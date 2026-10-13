//! Actual current admission selects positive originals before action/proof.
use super::{Context, Mode, PreparedInstanceTerminalError, Request, claims, retained::Original};
use recipe_application::{
    ActiveDeploymentEffectRepository, DeploymentError, DeploymentSnapshot, EffectClaim,
    PlannedResourceIdentity, PreparedInstanceDeleteHistory, PreparedInstanceDeleteHistoryState,
    PreparedInstanceDeleteRequest, PreparedInstanceDetachRequest, PreparedInstanceSafetyRepository,
    PreparedInstanceTerminalPreparationRepository, RemovalProgress, ResourceAction,
    ResourceOwnership, ResourceProgress,
};

pub(super) enum Admission {
    Detach(Box<PreparedInstanceDetachRequest>),
    Delete(Box<PreparedInstanceDeleteRequest>),
    Completed(Box<PreparedInstanceDeleteRequest>),
}
pub(super) async fn prepare(
    context: &Context,
    request: &Request,
    original: &mut Original,
) -> Result<Admission, PreparedInstanceTerminalError> {
    context.controller.validate_owners()?;
    request.remove.command.validate(
        &request.identity,
        recipe_application::DeploymentOperation::Remove,
    )?;
    if request.remove.deployment_id != original.request.remove.deployment_id
        || request.resource != original.request.resource
        || (request.mode == Mode::Original && request.remove != original.request.remove)
    {
        return Err(DeploymentError::IntentMismatch.into());
    }
    let mut recovered_detach = None;
    let mut recovered_delete = None;
    let snapshot = match request.mode {
        Mode::Original => {
            context
                .deployment
                .admit_remove(&request.identity, request.remove)
                .await?
                .snapshot
        }
        Mode::Resume => {
            let resume = context
                .deployment
                .admit_remove_resume(&request.identity, request.remove)
                .await?;
            recovered_detach = resume
                .instance_detach_recovery()
                .iter()
                .find(|entry| entry.request().effect().resource == request.resource)
                .map(|entry| entry.request().clone());
            recovered_delete = resume
                .instance_delete_history()
                .iter()
                .find(|entry| entry.original().original().effect().resource == request.resource)
                .cloned();
            resume.admission().snapshot.clone()
        }
    };
    qualify_snapshot(&snapshot, request, original)?;
    let progress = snapshot
        .resources
        .get(&request.resource)
        .ok_or(DeploymentError::Unavailable)?;
    if let Some(history) = recovered_delete {
        return from_history(context, request, original, &snapshot, &history);
    }
    if progress.removal == RemovalProgress::Deleted && progress.active_attempt.is_none() {
        if request.mode != Mode::Original {
            return Err(DeploymentError::ReconciliationRequired.into());
        }
        // The original command is not converted into a fake fresh admission.
        // Full positive protected original history supplies claim/Create/closure/CAS.
        let history = context
            .history
            .load_original_delete_history(
                &request.identity,
                &snapshot.intent,
                request.remove.command,
                &request.resource,
            )
            .await?;
        if history.original().original().effect().command != request.remove.command
            || history.state() != PreparedInstanceDeleteHistoryState::Applied
        {
            return Err(DeploymentError::ReconciliationRequired.into());
        }
        return from_history(context, request, original, &snapshot, &history);
    }
    prepare_active(context, request, original, snapshot, recovered_detach).await
}
async fn prepare_active(
    context: &Context,
    request: &Request,
    original: &mut Original,
    snapshot: DeploymentSnapshot,
    recovered_detach: Option<PreparedInstanceDetachRequest>,
) -> Result<Admission, PreparedInstanceTerminalError> {
    let progress = snapshot
        .resources
        .get(&request.resource)
        .ok_or(DeploymentError::Unavailable)?;
    let active = context
        .preparation
        .load_active_effect(
            &request.identity,
            request.remove.command,
            request.remove.deployment_id,
            &request.resource,
        )
        .await?;
    let action = next(progress, active.as_ref())?;
    let target = if let Some(detach) = &recovered_detach {
        detach.target().clone()
    } else if let Some(target) = &original.target {
        target.clone()
    } else {
        context
            .safety
            .load_original_target(
                &request.identity,
                request.remove.command,
                &snapshot.intent,
                &request.resource,
            )
            .await?
    };
    original.target(&target)?;
    let claim = match active {
        Some(claim) => {
            claims::restore_loaded(original, &claim)?;
            claim
        }
        None => claims::claim(context, request, original, &snapshot, action).await?,
    };
    let input = recipe_application::PreparedInstanceTerminalPreparationInput::new(
        snapshot.intent,
        target,
        claim,
        request.remove.command,
        context.remove.clone(),
    )?;
    match action {
        ResourceAction::Detach => {
            let checked = context
                .terminal
                .prepare_detach(&request.identity, &input)
                .await?;
            if recovered_detach.as_ref().is_some_and(|old| old != &checked) {
                return Err(DeploymentError::IntentMismatch.into());
            }
            original.detach(&checked)?;
            Ok(Admission::Detach(Box::new(checked)))
        }
        ResourceAction::Delete => {
            let checked = context
                .terminal
                .prepare_delete(&request.identity, &input)
                .await?;
            // Exact full request and original positive CAS retained BEFORE any writer await.
            original.delete(&checked)?;
            Ok(Admission::Delete(Box::new(checked)))
        }
        _ => Err(DeploymentError::InvalidAction.into()),
    }
}

fn qualify_snapshot(
    snapshot: &DeploymentSnapshot,
    request: &Request,
    original: &mut Original,
) -> Result<(), DeploymentError> {
    let plan = snapshot
        .intent
        .resources()
        .get(&request.resource)
        .ok_or(DeploymentError::Unavailable)?;
    if plan.ownership() != ResourceOwnership::Owned
        || !matches!(plan.identity(), PlannedResourceIdentity::Instance { .. })
    {
        return Err(DeploymentError::InvalidAction);
    }
    plan.validate_action(ResourceAction::Delete)?;
    if original
        .intent
        .as_ref()
        .is_some_and(|old| old != &snapshot.intent)
    {
        return Err(DeploymentError::IntentMismatch);
    }
    original.intent = Some(snapshot.intent.clone());
    Ok(())
}
fn from_history(
    context: &Context,
    request: &Request,
    original: &mut Original,
    snapshot: &DeploymentSnapshot,
    history: &PreparedInstanceDeleteHistory,
) -> Result<Admission, PreparedInstanceTerminalError> {
    let checked = history.original().original();
    if checked.intent() != &snapshot.intent
        || checked.effect().resource != request.resource
        || checked.current_command() != request.remove.command
        || checked.configuration() != &context.remove
        || snapshot.resources.get(&request.resource) != Some(history.observed_progress().progress())
    {
        return Err(DeploymentError::IntentMismatch.into());
    }
    original.target(checked.target())?;
    claims::restore_loaded(original, checked.effect())?;
    original.delete(checked)?;
    original.history(history)?;
    match history.state() {
        PreparedInstanceDeleteHistoryState::Active => {
            Ok(Admission::Delete(Box::new(checked.clone())))
        }
        PreparedInstanceDeleteHistoryState::Applied => {
            Ok(Admission::Completed(Box::new(checked.clone())))
        }
    }
}
pub(super) fn next(
    progress: &ResourceProgress,
    active: Option<&EffectClaim>,
) -> Result<ResourceAction, PreparedInstanceTerminalError> {
    if progress.active_attempt != active.map(|claim| claim.provenance.attempt_id) {
        return Err(DeploymentError::ReconciliationRequired.into());
    }
    if let Some(claim) = active {
        return match claim.action {
            ResourceAction::Detach | ResourceAction::Delete => Ok(claim.action),
            ResourceAction::Drain => Err(PreparedInstanceTerminalError::DrainPending),
            _ => Err(DeploymentError::InvalidAction.into()),
        };
    }
    match progress.removal {
        RemovalProgress::Draining => Ok(ResourceAction::Detach),
        RemovalProgress::Detached => Ok(ResourceAction::Delete),
        _ => Err(PreparedInstanceTerminalError::DrainPending),
    }
}
