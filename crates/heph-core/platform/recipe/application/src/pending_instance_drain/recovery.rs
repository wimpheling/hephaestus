use super::{PendingInstanceDrainClosure, validate_original};
use crate::{
    CommandIdentity, DeploymentError, DeploymentLifecycle, DeploymentOperation, DeploymentSnapshot,
    EffectClaim, InstallProgress, PreparedInstanceSafetyTarget, RemovalProgress,
};
impl PendingInstanceDrainClosure {
    /// Compares advanced `RecoveryRequired` progress without rewriting the old claim.
    ///
    /// Native142 must verify contiguous protected history, exact current native
    /// generation and original/publisher associations. No absent closure, expired
    /// claim or version advancement grants absence or bootstrap authority.
    ///
    /// # Errors
    /// Rejects non-RecoveryRequired lifecycle/removal, changed active attempt or
    /// originals, absent safe diagnostic or current version not greater than old.
    pub fn with_recovery_progress(
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
            || drain.generation > i64::MAX.cast_unsigned()
            || snapshot.version == 0
            || snapshot.version > i64::MAX.cast_unsigned()
            || snapshot.lifecycle != DeploymentLifecycle::RecoveryRequired
            || progress.install != InstallProgress::Ready
            || progress.active_attempt != Some(drain.provenance.attempt_id)
            || progress.version <= drain.resource_version
            || progress.version > i64::MAX.cast_unsigned()
            || progress.removal != RemovalProgress::RecoveryRequired
            || progress.diagnostic.is_none()
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
}
