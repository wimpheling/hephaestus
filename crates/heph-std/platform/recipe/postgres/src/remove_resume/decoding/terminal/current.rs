//! Full checked originals/current State, independent of recorded publisher.
use super::{Loaded, Native143};
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentSnapshot, OriginalInstanceClosure,
    PreparedInstanceDeleteHistory, PreparedInstanceDeleteRequest, PreparedInstanceDetachRecovery,
    PreparedInstanceDetachRequest, PreparedInstanceTerminalProgress,
};

#[derive(Default)]
pub(in crate::remove_resume::decoding) struct Histories {
    pub detach: Vec<PreparedInstanceDetachRecovery>,
    pub terminal: Vec<PreparedInstanceDeleteHistory>,
}
pub(in crate::remove_resume::decoding) fn assemble(
    decoded: &Native143,
    closures: &[OriginalInstanceClosure],
    snapshot: &DeploymentSnapshot,
    command: CommandIdentity,
) -> Result<Histories, DeploymentError> {
    if decoded.is_empty() {
        return Ok(Histories::default());
    }
    let loaded = decoded
        .loaded
        .as_ref()
        .ok_or(DeploymentError::ReconciliationRequired)?;
    let mut result = Histories::default();
    for item in &loaded.detach {
        let request = PreparedInstanceDetachRequest::new(
            snapshot.intent.clone(),
            item.target.clone(),
            closure(item, closures)?.clone(),
            item.claim.clone(),
            command,
            loaded.config.clone(),
        )?;
        let recovery = PreparedInstanceDetachRecovery::new(request, progress(item)?)?;
        if snapshot.resources.get(&item.claim.resource)
            != Some(recovery.observed_progress().progress())
        {
            return Err(DeploymentError::IntentMismatch);
        }
        result.detach.push(recovery);
    }
    for item in &loaded.terminal {
        let request = PreparedInstanceDeleteRequest::new(
            snapshot.intent.clone(),
            item.target.clone(),
            closure(item, closures)?.clone(),
            item.claim.clone(),
            command,
            loaded.config.clone(),
            item.wire
                .before
                .ok_or(DeploymentError::ReconciliationRequired)?,
        )?;
        let history = PreparedInstanceDeleteHistory::from_observed_parts(
            request,
            progress(item)?,
            item.wire.operation,
            item.wire.after,
            item.publisher.clone(),
        )?;
        if snapshot.resources.get(&item.claim.resource)
            != Some(history.observed_progress().progress())
        {
            return Err(DeploymentError::IntentMismatch);
        }
        result.terminal.push(history);
    }
    Ok(result)
}
fn closure<'a>(
    item: &Loaded,
    closures: &'a [OriginalInstanceClosure],
) -> Result<&'a OriginalInstanceClosure, DeploymentError> {
    let mut found = closures
        .iter()
        .filter(|closure| closure.drain.resource == item.claim.resource);
    let result = found
        .next()
        .ok_or(DeploymentError::ReconciliationRequired)?;
    if found.next().is_some() {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(result)
}
fn progress(item: &Loaded) -> Result<PreparedInstanceTerminalProgress, DeploymentError> {
    PreparedInstanceTerminalProgress::new(
        item.wire.state.progress()?,
        crate::hydration::unsigned(item.wire.state.generation)?,
        item.wire.state.last_action,
    )
}

pub(in crate::remove_resume::decoding) fn compare_replay(
    saved: &Native143,
    current: &Native143,
) -> Result<(), DeploymentError> {
    if saved.is_empty() && current.is_empty() {
        return Ok(());
    }
    let old = saved
        .loaded
        .as_ref()
        .ok_or(DeploymentError::ReconciliationRequired)?;
    let new = current
        .loaded
        .as_ref()
        .ok_or(DeploymentError::ReconciliationRequired)?;
    if old.config != new.config {
        return Err(DeploymentError::IntentMismatch);
    }
    for item in &old.detach {
        let actual = new
            .detach
            .iter()
            .find(|actual| actual.claim == item.claim)
            .ok_or(DeploymentError::ReconciliationRequired)?;
        if actual.target != item.target {
            return Err(DeploymentError::IntentMismatch);
        }
    }
    for item in &old.terminal {
        let actual = new
            .terminal
            .iter()
            .find(|actual| actual.claim == item.claim)
            .ok_or(DeploymentError::ReconciliationRequired)?;
        if actual.target != item.target
            || actual.wire.before != item.wire.before
            || (item.wire.operation.is_some()
                && (actual.wire.operation != item.wire.operation
                    || actual.wire.after != item.wire.after
                    || actual.publisher != item.publisher))
        {
            return Err(DeploymentError::IntentMismatch);
        }
    }
    // A current admission may not introduce unrelated terminal originals on an old
    // logical replay. Each positive publication remains native-qualified separately.
    if old.detach.len() != new.detach.len() || old.terminal.len() != new.terminal.len() {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}
