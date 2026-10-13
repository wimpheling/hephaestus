use crate::{
    DeploymentError, EffectClaim, InstallProgress, RemovalProgress, ResourceAction,
    ResourceProgress,
};

/// All seven native State fields, retaining native generation and prior action.
///
/// Pure comparisons cannot prove contiguous protected history. Native143 must
/// qualify the complete same-original prefix and actual predecessor independently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceTerminalProgress {
    progress: ResourceProgress,
    generation: u64,
    last_action: Option<ResourceAction>,
}
impl PreparedInstanceTerminalProgress {
    /// Retains complete bounded Ready progress without dropping diagnostics/attempts.
    ///
    /// # Errors
    /// Rejects zero/out-of-bigint versions/generation, non-Ready installation and
    /// unsupported prior actions. Unknown wire fields must be denied by adapters.
    pub fn new(
        progress: ResourceProgress,
        generation: u64,
        last_action: Option<ResourceAction>,
    ) -> Result<Self, DeploymentError> {
        if progress.install != InstallProgress::Ready
            || !positive(progress.version)
            || !positive(generation)
            || !matches!(
                last_action,
                Some(ResourceAction::Drain | ResourceAction::Detach | ResourceAction::Delete)
            )
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(Self {
            progress,
            generation,
            last_action,
        })
    }
    pub(super) fn active(&self, effect: &EffectClaim) -> Result<(), DeploymentError> {
        let (prior, removal) = match effect.action {
            ResourceAction::Detach => (ResourceAction::Drain, RemovalProgress::Draining),
            ResourceAction::Delete => (ResourceAction::Detach, RemovalProgress::Detached),
            _ => return Err(DeploymentError::InvalidAction),
        };
        let endpoint = if self.progress.version == effect.resource_version {
            self.progress.removal == removal && self.progress.diagnostic.is_none()
        } else {
            self.progress.version > effect.resource_version
                && self.progress.removal == RemovalProgress::RecoveryRequired
                && self.progress.diagnostic.is_some()
        };
        if self.generation != effect.generation
            || self.last_action != Some(prior)
            || self.progress.active_attempt != Some(effect.provenance.attempt_id)
            || !endpoint
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(())
    }
    pub(super) fn applied(&self, effect: &EffectClaim) -> Result<(), DeploymentError> {
        if effect.action != ResourceAction::Delete
            || self.generation != effect.generation
            || self.last_action != Some(ResourceAction::Delete)
            || self.progress.active_attempt.is_some()
            || self.progress.diagnostic.is_some()
            || self.progress.removal != RemovalProgress::Deleted
            || self.progress.version <= effect.resource_version
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(())
    }
    /// Returns all five public State fields unchanged.
    #[must_use]
    pub const fn progress(&self) -> &ResourceProgress {
        &self.progress
    }
    /// Returns exact native execution generation, never inferred from a version.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    /// Returns the native original last action, not a rewritten current action.
    #[must_use]
    pub const fn last_action(&self) -> Option<ResourceAction> {
        self.last_action
    }
}
const fn positive(value: u64) -> bool {
    value > 0 && value <= i64::MAX.cast_unsigned()
}
