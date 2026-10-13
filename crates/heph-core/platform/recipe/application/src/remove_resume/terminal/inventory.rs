use crate::{
    DeploymentAdmission, DeploymentError, DeploymentLifecycle, EffectClaim,
    OriginalInstanceClosure, PendingInstanceDrainClosure, PreparedInstanceDeleteHistory,
    PreparedInstanceDeleteHistoryState, PreparedInstanceDetachRecovery, RemovalProgress,
    RemoveContinuation, RemoveResumeConfiguration, ResourceAction,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn current(
    admission: &DeploymentAdmission,
    continuations: &[RemoveContinuation],
) -> Result<(), DeploymentError> {
    let snapshot = &admission.snapshot;
    if admission.receipt.command.operation() != crate::DeploymentOperation::Remove {
        return Err(DeploymentError::IntentMismatch);
    }
    if admission.receipt.deployment_id != snapshot.intent.id()
        || snapshot.resources.len() != snapshot.intent.resources().len()
        || snapshot.version == 0
        || snapshot.version > i64::MAX.cast_unsigned()
        || !snapshot
            .intent
            .resources()
            .keys()
            .all(|name| snapshot.resources.contains_key(name))
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let mut names = BTreeSet::new();
    let mut attempts = BTreeSet::new();
    for item in continuations {
        let claim = &item.claim;
        if claim.command == admission.receipt.command
            || !names.insert(&claim.resource)
            || !attempts.insert(claim.provenance.attempt_id)
            || claim.generation == 0
            || claim.generation > i64::MAX.cast_unsigned()
            || claim.resource_version == 0
            || claim.resource_version > i64::MAX.cast_unsigned()
            || snapshot.resources.get(&claim.resource).is_none_or(|state| {
                state.active_attempt != Some(claim.provenance.attempt_id)
                    || state.version < claim.resource_version
            })
            || !matches!(
                snapshot.lifecycle,
                DeploymentLifecycle::Removing | DeploymentLifecycle::RecoveryRequired
            )
        {
            return Err(DeploymentError::IntentMismatch);
        }
    }
    for (name, state) in &snapshot.resources {
        if state.version == 0
            || state.version > i64::MAX.cast_unsigned()
            || state.active_attempt.is_some() != names.contains(name)
            || (state.removal == RemovalProgress::RecoveryRequired
                && snapshot.lifecycle != DeploymentLifecycle::RecoveryRequired)
        {
            return Err(DeploymentError::ReconciliationRequired);
        }
    }
    Ok(())
}

pub(super) fn entries(
    admission: &DeploymentAdmission,
    continuations: &[RemoveContinuation],
    closures: &[OriginalInstanceClosure],
    pending: &[PendingInstanceDrainClosure],
    detach: &[PreparedInstanceDetachRecovery],
    terminal: &[PreparedInstanceDeleteHistory],
    configuration: &RemoveResumeConfiguration,
) -> Result<(), DeploymentError> {
    let maximum = admission
        .snapshot
        .intent
        .resources()
        .values()
        .filter(|plan| {
            matches!(
                plan.identity(),
                crate::PlannedResourceIdentity::Instance { .. }
            )
        })
        .count();
    if detach.is_empty() && terminal.is_empty() || detach.len() + terminal.len() > maximum {
        return Err(DeploymentError::IntentMismatch);
    }
    let mut blocked: BTreeSet<_> = pending.iter().map(|item| &item.drain().resource).collect();
    blocked.extend(
        continuations
            .iter()
            .filter(|item| {
                item.claim.action == ResourceAction::Drain
                    && admission.snapshot.resources[&item.claim.resource].removal
                        == RemovalProgress::RecoveryRequired
            })
            .map(|item| &item.claim.resource),
    );
    let mut names = BTreeSet::new();
    let mut attempts = BTreeMap::new();
    for item in closures {
        track(&mut attempts, &item.drain)?;
    }
    for item in continuations {
        track(&mut attempts, &item.claim)?;
    }
    for item in detach {
        let request = item.request();
        let claim = request.effect();
        if blocked.contains(&claim.resource)
            || !names.insert(&claim.resource)
            || request.intent() != &admission.snapshot.intent
            || request.current_command() != admission.receipt.command
            || request.configuration() != configuration
            || admission.snapshot.resources.get(&claim.resource)
                != Some(item.observed_progress().progress())
            || !closures.contains(request.closure())
        {
            return Err(DeploymentError::IntentMismatch);
        }
        super::active_match(
            continuations,
            claim,
            request.closure(),
            PreparedInstanceDeleteHistoryState::Active,
        )?;
        track(&mut attempts, claim)?;
    }
    for item in terminal {
        let request = item.original().original();
        let claim = request.effect();
        if blocked.contains(&claim.resource)
            || !names.insert(&claim.resource)
            || request.intent() != &admission.snapshot.intent
            || request.current_command() != admission.receipt.command
            || request.configuration() != configuration
            || admission.snapshot.resources.get(&claim.resource)
                != Some(item.observed_progress().progress())
            || !closures.contains(request.closure())
        {
            return Err(DeploymentError::IntentMismatch);
        }
        super::active_match(continuations, claim, request.closure(), item.state())?;
        track(&mut attempts, claim)?;
    }
    for item in continuations {
        let needs_entry = item.claim.action == ResourceAction::Delete
            || (item.claim.action == ResourceAction::Detach
                && matches!(
                    item.claim.identity,
                    crate::PlannedResourceIdentity::Instance { .. }
                )
                && admission.snapshot.intent.resources()[&item.claim.resource].removal()
                    == recipe_domain::RemovalPolicy::Delete
                && admission.snapshot.resources[&item.claim.resource].removal
                    == RemovalProgress::RecoveryRequired);
        if needs_entry && !names.contains(&item.claim.resource) {
            return Err(DeploymentError::ReconciliationRequired);
        }
    }
    Ok(())
}
fn track(
    attempts: &mut BTreeMap<crate::DeploymentAttemptId, EffectClaim>,
    claim: &EffectClaim,
) -> Result<(), DeploymentError> {
    if attempts
        .insert(claim.provenance.attempt_id, claim.clone())
        .is_some_and(|old| old != *claim)
    {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(())
}
