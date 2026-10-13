//! Checked pending comparison data, never evidence that a closure is absent.
mod recovery;
use crate::{
    CommandIdentity, DeploymentError, DeploymentLifecycle, DeploymentOperation, DeploymentSnapshot,
    EffectClaim, PreparedInstanceSafetyTarget, RemovalProgress, ResourceAction, ResourceOwnership,
};

/// An exact original active Drain awaiting independent native closure qualification.
///
/// Construction observes no database, checks no current rights and grants no IO.
/// The native adapter must qualify the complete original inventory before admitting
/// this projection, including durable generation and both lease association directions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingInstanceDrainClosure {
    snapshot: DeploymentSnapshot,
    current_command: CommandIdentity,
    target: PreparedInstanceSafetyTarget,
    drain: EffectClaim,
}

impl PendingInstanceDrainClosure {
    /// Compares whole intent, original Create pins and active original Drain progress.
    ///
    /// No missing row, expiry, absent closure or caller-supplied snapshot proves absence.
    ///
    /// # Errors
    /// Rejects foreign pins, a substituted claim, unsupported state or current command.
    pub fn new(
        snapshot: DeploymentSnapshot,
        current_command: CommandIdentity,
        target: PreparedInstanceSafetyTarget,
        drain: EffectClaim,
    ) -> Result<Self, DeploymentError> {
        validate_original(&snapshot, &target, &drain)?;
        let progress = snapshot
            .resources
            .get(&drain.resource)
            .ok_or(DeploymentError::IntentMismatch)?;
        if current_command.operation() != DeploymentOperation::Remove
            || current_command == drain.command
            || snapshot.version == 0
            || !matches!(
                snapshot.lifecycle,
                DeploymentLifecycle::Removing | DeploymentLifecycle::RecoveryRequired
            )
            || progress.active_attempt != Some(drain.provenance.attempt_id)
            || progress.version != drain.resource_version
            || !matches!(
                progress.removal,
                RemovalProgress::Draining | RemovalProgress::RecoveryRequired
            )
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(Self {
            snapshot,
            current_command,
            target,
            drain,
        })
    }

    /// Returns the whole compared snapshot, not native admission or physical proof.
    #[must_use]
    pub const fn snapshot(&self) -> &DeploymentSnapshot {
        &self.snapshot
    }

    /// Returns the current Remove command; never original actor credentials.
    #[must_use]
    pub const fn current_command(&self) -> CommandIdentity {
        self.current_command
    }

    /// Returns the complete original Create target and execution comparison data.
    #[must_use]
    pub const fn target(&self) -> &PreparedInstanceSafetyTarget {
        &self.target
    }

    /// Returns the exact original active Drain, including its unchanged provenance.
    #[must_use]
    pub const fn drain(&self) -> &EffectClaim {
        &self.drain
    }
}

fn validate_original(
    snapshot: &DeploymentSnapshot,
    target: &PreparedInstanceSafetyTarget,
    drain: &EffectClaim,
) -> Result<(), DeploymentError> {
    let original = target.original();
    let plan = snapshot
        .intent
        .resources()
        .get(&drain.resource)
        .ok_or(DeploymentError::IntentMismatch)?;
    if snapshot.intent.id() != original.deployment_id
        || snapshot.intent.project_id() != target.prepared().project_id()
        || plan.ownership() != ResourceOwnership::Owned
        || plan.identity() != original.identity
        || plan.input_hash() != original.input_hash
        || drain.deployment_id != original.deployment_id
        || drain.resource != original.resource
        || drain.identity != original.identity
        || drain.input_hash != original.input_hash
        || drain.command.operation() != DeploymentOperation::Remove
        || drain.action != ResourceAction::Drain
        || drain.provenance.actor_id != drain.command.actor_id()
        || drain.provenance.request_id.as_uuid().is_nil()
        || drain.generation == 0
        || drain.resource_version == 0
    {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(())
}
