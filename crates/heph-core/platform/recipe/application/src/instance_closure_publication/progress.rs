use crate::{
    DeploymentError, DeploymentOperation, EffectClaim, InstallProgress, PlannedResourceIdentity,
    RemovalProgress, ResourceAction, ResourceProgress,
};

/// Exact per-resource progress recorded in the original Pending admission.
///
/// No vanished whole historical snapshot is reconstructed. Native generation and
/// `last_action` remain separate because `ResourceProgress` has neither field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceClosurePublicationProgress {
    drain: EffectClaim,
    progress: ResourceProgress,
    generation: u64,
    last_action: Option<ResourceAction>,
}
impl InstanceClosurePublicationProgress {
    /// Compares original active Drain to actual recorded Pending state.
    ///
    /// # Errors
    /// Rejects changed attempt/generation, non-Ready/Create history, invalid original
    /// provenance or unsupported Draining/advanced-RecoveryRequired progress.
    pub fn new(
        drain: EffectClaim,
        progress: ResourceProgress,
        generation: u64,
        last_action: Option<ResourceAction>,
    ) -> Result<Self, DeploymentError> {
        let pending = match progress.removal {
            RemovalProgress::Draining => {
                progress.version == drain.resource_version && progress.diagnostic.is_none()
            }
            RemovalProgress::RecoveryRequired => {
                progress.version > drain.resource_version && progress.diagnostic.is_some()
            }
            _ => false,
        };
        if !matches!(drain.identity, PlannedResourceIdentity::Instance { .. })
            || drain.action != ResourceAction::Drain
            || drain.command.operation() != DeploymentOperation::Remove
            || drain.provenance.actor_id != drain.command.actor_id()
            || drain.provenance.request_id.as_uuid().is_nil()
            || drain.resource_version == 0
            || drain.resource_version > i64::MAX.cast_unsigned()
            || generation == 0
            || generation > i64::MAX.cast_unsigned()
            || generation != drain.generation
            || progress.version == 0
            || progress.version > i64::MAX.cast_unsigned()
            || progress.active_attempt != Some(drain.provenance.attempt_id)
            || progress.install != InstallProgress::Ready
            || last_action != Some(ResourceAction::Create)
            || !pending
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(Self {
            drain,
            progress,
            generation,
            last_action,
        })
    }
    /// Returns full unchanged Drain comparison data, not credentials.
    #[must_use]
    pub const fn drain(&self) -> &EffectClaim {
        &self.drain
    }
    /// Returns full originally observed progress, not present-day resource state.
    #[must_use]
    pub const fn progress(&self) -> &ResourceProgress {
        &self.progress
    }
    /// Returns exact native `execution_generation` from original admission evidence.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    /// Returns original `last_action`, never replaced with Drain.
    #[must_use]
    pub const fn last_action(&self) -> Option<ResourceAction> {
        self.last_action
    }
}
