use super::{InstanceClosurePublicationProgress, OriginalInstanceClosure};
use crate::{
    CommandIdentity, DeploymentCommandId, DeploymentError, DeploymentOperation, EffectClaim,
};
use identity_domain::RequestId;

/// Checked fields of native142's `qualified_fresh_remove` publication DTO.
///
/// Recorded publisher B is data, independent of current cleanup user C. Native
/// admission/originals/config/events must be independently qualified. This value
/// never authenticates publisher B or original Drain user A.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedFreshInstanceClosurePublication {
    drain: EffectClaim,
    command: CommandIdentity,
    native_command_id: DeploymentCommandId,
    admission_request: RequestId,
    publication_request: RequestId,
    admission_version: u64,
    pending: InstanceClosurePublicationProgress,
}
impl QualifiedFreshInstanceClosurePublication {
    /// Compares full old Drain, actual fresh command and recorded Pending pins.
    ///
    /// Admission/publication requests may differ on logical retry; both nonnil
    /// originals are retained. The native ID must match typed actor/idempotency
    /// derivation. Construction establishes no actual database admission.
    ///
    /// # Errors
    /// Rejects substituted Drain/progress, old/non-Remove command, wrong native ID,
    /// nil provenance or nonpositive/out-of-bigint admission version.
    pub fn new(
        drain: EffectClaim,
        command: CommandIdentity,
        native_command_id: DeploymentCommandId,
        admission_request: RequestId,
        publication_request: RequestId,
        admission_version: u64,
        pending: InstanceClosurePublicationProgress,
    ) -> Result<Self, DeploymentError> {
        if drain != *pending.drain()
            || command.operation() != DeploymentOperation::Remove
            || command == drain.command
            || native_command_id != command.id()
            || admission_request.as_uuid().is_nil()
            || publication_request.as_uuid().is_nil()
            || admission_version == 0
            || admission_version > i64::MAX.cast_unsigned()
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(Self {
            drain,
            command,
            native_command_id,
            admission_request,
            publication_request,
            admission_version,
            pending,
        })
    }
    pub(super) fn validate(
        &self,
        closure: &OriginalInstanceClosure,
    ) -> Result<(), DeploymentError> {
        if self.drain != closure.drain
            || closure.actor != self.command.actor_id()
            || closure.request != self.publication_request
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(())
    }
    /// Returns unchanged Drain, never a saved principal to authenticate.
    #[must_use]
    pub const fn drain(&self) -> &EffectClaim {
        &self.drain
    }
    /// Returns recorded publisher command, distinct from current cleanup's command.
    #[must_use]
    pub const fn command(&self) -> CommandIdentity {
        self.command
    }
    /// Returns exact native ID already compared with typed command derivation.
    #[must_use]
    pub const fn native_command_id(&self) -> DeploymentCommandId {
        self.native_command_id
    }
    /// Returns first fresh admission request, not a live request credential.
    #[must_use]
    pub const fn admission_request(&self) -> RequestId {
        self.admission_request
    }
    /// Returns actual publication request, separately from first admission.
    #[must_use]
    pub const fn publication_request(&self) -> RequestId {
        self.publication_request
    }
    /// Returns originally admitted deployment CAS, never rebuilt from current state.
    #[must_use]
    pub const fn admission_version(&self) -> u64 {
        self.admission_version
    }
    /// Returns recorded per-resource Pending progress without a whole snapshot.
    #[must_use]
    pub const fn pending(&self) -> &InstanceClosurePublicationProgress {
        &self.pending
    }
}
