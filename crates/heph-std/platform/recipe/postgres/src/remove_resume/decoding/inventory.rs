//! Complete inventory and current snapshot binding, never a historical rebuild.
use super::{Decoded, Roots, pending, wire};
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentLifecycle, DeploymentSnapshot,
    PendingInstanceDrainClosure, PlannedResourceIdentity, RemovalProgress,
};
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) fn endpoints(
    decoded: &mut Decoded,
    data: &Value,
    roots: Option<&Roots<'_>>,
) -> Result<(), DeploymentError> {
    let bound = data["claims"]
        .as_array()
        .ok_or(DeploymentError::ReconciliationRequired)?
        .len();
    for entry in &decoded.continuations {
        let plan = roots.and_then(|roots| roots.intent.resources().get(&entry.claim.resource));
        let terminal_claim = entry.claim.action == recipe_application::ResourceAction::Delete
            && decoded.terminal.terminal.iter().any(|item| {
                crate::execution_rows::ClaimWire::new(&entry.claim)
                    .is_ok_and(|claim| item.claim == claim)
            });
        if !matches!(
            entry.claim.action,
            recipe_application::ResourceAction::Drain
                | recipe_application::ResourceAction::Detach
                | recipe_application::ResourceAction::Retain
        ) && !terminal_claim
            || plan.is_some_and(|plan| {
                plan.ownership() != recipe_application::ResourceOwnership::Owned
            })
        {
            return Err(DeploymentError::IntentMismatch);
        }
    }
    let pending_values = wire::optional(data, "pending", bound)?;
    let recovery_values = wire::optional(data, "recovery", bound)?;
    if pending_values.is_empty() && recovery_values.is_empty() {
        return Ok(());
    }
    let roots = roots.ok_or(DeploymentError::ReconciliationRequired)?;
    let mut resources = BTreeSet::new();
    for value in pending_values {
        let item = pending::decode(value, roots)?;
        if !resources.insert(item.drain.resource.clone())
            || decoded
                .closures
                .iter()
                .any(|closure| closure.drain.resource == item.drain.resource)
            || !decoded
                .continuations
                .iter()
                .any(|entry| entry.claim == item.drain && entry.closure.is_none())
        {
            return Err(DeploymentError::IntentMismatch);
        }
        decoded.pending.push(item);
    }
    for value in recovery_values {
        wire::fields(value, &["resource", "drain_claim", "observed_progress"])?;
        let resource = value["resource"]
            .as_str()
            .ok_or(DeploymentError::IntentMismatch)?;
        let closure = decoded
            .closures
            .iter()
            .find(|closure| closure.drain.resource.as_str() == resource)
            .ok_or(DeploymentError::IntentMismatch)?;
        if !resources.insert(closure.drain.resource.clone())
            || serde_json::to_value(crate::execution_rows::ClaimWire::new(&closure.drain)?)
                .map_err(|_| DeploymentError::Serialization)?
                != value["drain_claim"]
            || !decoded.continuations.iter().any(|entry| {
                entry.claim == closure.drain && entry.closure.as_ref() == Some(closure)
            })
        {
            return Err(DeploymentError::IntentMismatch);
        }
        let state = wire::state(&value["observed_progress"])?;
        if state.removal != RemovalProgress::RecoveryRequired {
            return Err(DeploymentError::IntentMismatch);
        }
        pending::progress(&closure.drain, &state)?;
        decoded.recovery.push((closure.drain.clone(), state));
    }
    Ok(())
}

pub struct Current {
    pub continuations: Vec<recipe_application::RemoveContinuation>,
    pub closures: Vec<recipe_application::OriginalInstanceClosure>,
    pub pending: Vec<PendingInstanceDrainClosure>,
    pub detach: Vec<recipe_application::PreparedInstanceDetachRecovery>,
    pub terminal: Vec<recipe_application::PreparedInstanceDeleteHistory>,
}

pub fn current(
    decoded: Decoded,
    snapshot: &DeploymentSnapshot,
    command: CommandIdentity,
) -> Result<Current, DeploymentError> {
    if snapshot.version == 0 || snapshot.version > i64::MAX.cast_unsigned() {
        return Err(DeploymentError::IntentMismatch);
    }
    let histories =
        super::terminal::assemble(&decoded.terminal, &decoded.closures, snapshot, command)?;
    let mut active = BTreeSet::new();
    for entry in &decoded.continuations {
        let progress = snapshot
            .resources
            .get(&entry.claim.resource)
            .ok_or(DeploymentError::IntentMismatch)?;
        if !active.insert(entry.claim.resource.clone())
            || progress.active_attempt != Some(entry.claim.provenance.attempt_id)
            || progress.version < entry.claim.resource_version
            || progress.version > i64::MAX.cast_unsigned()
            || entry.closure.as_ref().is_some_and(|closure| {
                closure.drain != entry.claim && !decoded.terminal.matches_active(&entry.claim)
            })
            || (matches!(
                entry.claim.identity,
                PlannedResourceIdentity::Instance { .. }
            ) && entry.closure.is_none()
                && !decoded.pending.iter().any(|item| item.drain == entry.claim))
        {
            return Err(DeploymentError::IntentMismatch);
        }
    }
    if snapshot
        .resources
        .iter()
        .any(|(name, progress)| progress.active_attempt.is_some() != active.contains(name))
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let mut pending_values = Vec::new();
    for item in decoded.pending {
        if snapshot.resources.get(&item.drain.resource) != Some(&item.state.progress()?) {
            return Err(DeploymentError::IntentMismatch);
        }
        let checked = if item.state.removal == RemovalProgress::RecoveryRequired {
            PendingInstanceDrainClosure::with_recovery_progress(
                snapshot.clone(),
                command,
                item.target,
                item.drain,
            )?
        } else {
            PendingInstanceDrainClosure::new(snapshot.clone(), command, item.target, item.drain)?
        };
        pending_values.push(checked);
    }
    for (drain, state) in decoded.recovery {
        if snapshot.lifecycle != DeploymentLifecycle::RecoveryRequired
            || snapshot.resources.get(&drain.resource) != Some(&state.progress()?)
        {
            return Err(DeploymentError::IntentMismatch);
        }
    }
    Ok(Current {
        continuations: decoded.continuations,
        closures: decoded.closures,
        pending: pending_values,
        detach: histories.detach,
        terminal: histories.terminal,
    })
}
