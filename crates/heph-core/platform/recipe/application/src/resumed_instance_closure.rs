//! Fresh publisher input retaining the original Drain's stable removal identities.
use crate::{
    CommandIdentity, DeploymentError, DeploymentIntent, EffectClaim, PendingInstanceDrainClosure,
    PreparedInstanceClosureRequest, PreparedInstanceSafetyTarget,
};
use release_domain::{InstanceRemovalId, ReleaseCommandKey};

/// Comparison input for protected resumed publication, not an admission or receipt.
///
/// The current authenticated caller must independently match `current_command`.
/// Original claim provenance remains data; it must never populate actor credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumedPreparedInstanceClosureRequest {
    current_command: CommandIdentity,
    original: PreparedInstanceClosureRequest,
    pending: PendingInstanceDrainClosure,
}

impl ResumedPreparedInstanceClosureRequest {
    /// Retains exact original identities with an explicit current instance CAS.
    ///
    /// # Errors
    /// Rejects any changed intent, original target/claim, current command or invalid CAS.
    pub fn new(
        current_command: CommandIdentity,
        intent: DeploymentIntent,
        target: PreparedInstanceSafetyTarget,
        original_drain: EffectClaim,
        pending: PendingInstanceDrainClosure,
        expected_instance_version: u64,
    ) -> Result<Self, DeploymentError> {
        if current_command != pending.current_command()
            || intent != pending.snapshot().intent
            || target != *pending.target()
            || original_drain != *pending.drain()
        {
            return Err(DeploymentError::IntentMismatch);
        }
        let original = PreparedInstanceClosureRequest::new(
            intent,
            target,
            original_drain,
            expected_instance_version,
        )?;
        Ok(Self {
            current_command,
            original,
            pending,
        })
    }

    /// Returns only the current caller's Remove identity to authenticate afresh.
    #[must_use]
    pub const fn current_command(&self) -> CommandIdentity {
        self.current_command
    }

    /// Returns the checked full immutable intent.
    #[must_use]
    pub const fn intent(&self) -> &DeploymentIntent {
        self.original.intent()
    }

    /// Returns the whole original Create target, not historical credentials.
    #[must_use]
    pub const fn target(&self) -> &PreparedInstanceSafetyTarget {
        self.original.target()
    }

    /// Returns the unchanged original active Drain claim for native requalification.
    #[must_use]
    pub const fn drain(&self) -> &EffectClaim {
        self.original.drain()
    }

    /// Returns complete pending comparison data; no absence or cleanup is inferred.
    #[must_use]
    pub const fn pending(&self) -> &PendingInstanceDrainClosure {
        &self.pending
    }

    /// Returns actual admitted deployment version comparison data for native CAS.
    #[must_use]
    pub const fn expected_deployment_version(&self) -> u64 {
        self.pending.snapshot().version
    }

    /// Returns the caller's explicit positive instance CAS, bounded to signed bigint.
    #[must_use]
    pub const fn expected_instance_version(&self) -> u64 {
        self.original.expected_instance_version()
    }

    /// Returns the removal UUID derived from the original instance and Drain attempt.
    #[must_use]
    pub const fn removal_id(&self) -> InstanceRemovalId {
        self.original.removal_id()
    }

    /// Returns the stable key derived from the original Drain, not the fresh publisher.
    #[must_use]
    pub const fn command_key(&self) -> ReleaseCommandKey {
        self.original.command_key()
    }
}
