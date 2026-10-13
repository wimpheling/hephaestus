//! Exact original-removal comparison data; neither credentials nor cleanup proof.
use run_domain::{LegacyVmPlacement, Run, RunCleanupTarget};
use runtime_types::{AgentInstanceId, RunId};
use uuid::Uuid;

use crate::RepositoryError;

/// Immutable original closure fields, supplied by trusted owning composition.
/// Historical actor/request identifiers are data, never authenticated identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCleanupWakeOriginal {
    /// Original Drain effect attempt.
    pub drain_attempt: Uuid,
    /// Original logical Drain command.
    pub drain_command: Uuid,
    /// Exact persisted permanent removal request.
    pub removal_id: Uuid,
    /// Original immutable release command key.
    pub command_key: [u8; 32],
    /// Original removal input fingerprint.
    pub input_hash: [u8; 32],
    /// Original instance CAS before permanent closure.
    pub expected_version: u64,
    /// Original request creator, retained only for comparison.
    pub creator: Uuid,
    /// Original authenticated request identifier, retained only for comparison.
    pub request: Uuid,
}

/// Checked exact-Run comparison request; shape validation grants no IO authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCleanupWakeRequest {
    run_id: RunId,
    instance_id: AgentInstanceId,
    original: RunCleanupWakeOriginal,
}
impl RunCleanupWakeRequest {
    /// Checks comparison identifiers without choosing a profile, VM or owner.
    ///
    /// # Errors
    /// Rejects nil original identifiers or an invalid original CAS.
    pub fn new(
        run_id: RunId,
        instance_id: AgentInstanceId,
        original: RunCleanupWakeOriginal,
    ) -> Result<Self, RepositoryError> {
        if [
            run_id.as_uuid(),
            instance_id.as_uuid(),
            original.drain_attempt,
            original.drain_command,
            original.removal_id,
            original.creator,
            original.request,
        ]
        .iter()
        .any(Uuid::is_nil)
            || original.expected_version == 0
        {
            return Err(RepositoryError::InvalidData(
                "invalid original cleanup comparison",
            ));
        }
        Ok(Self {
            run_id,
            instance_id,
            original,
        })
    }
    /// Exact selected Run, not an inferred VM identifier.
    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }
    /// Exact instance covered by original permanent closure.
    #[must_use]
    pub const fn instance_id(&self) -> AgentInstanceId {
        self.instance_id
    }
    /// Original immutable closure comparison data.
    #[must_use]
    pub const fn original(&self) -> &RunCleanupWakeOriginal {
        &self.original
    }
}

/// Immutable backend-owned profile/placement comparison; no caller selects it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunCleanupWakeBinding {
    /// Positively admitted consumed strict Legacy placement.
    Legacy(Box<LegacyVmPlacement>),
    /// Positively admitted canonical owner and planned VM.
    Owned {
        /// Exact persisted generation, complete lease set and positive VM owner.
        target: Box<RunCleanupTarget>,
    },
}

/// Worker qualification result, returned only for permanently closed work.
///
/// The adapter must independently prove original request/inbox and durable Cancel
/// correlation, acquisition closure, immutable producer/profile/Run pins and
/// positive placement. This DTO alone is no persisted or physical proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCleanupWakeSnapshot {
    /// Independently matched original request, compared again by the controller.
    pub request: RunCleanupWakeRequest,
    /// Actual current Run, including durable cancellation.
    pub run: Run,
    /// Positive immutable owning binding.
    pub binding: RunCleanupWakeBinding,
}

/// Durable progress only; callers still verify action-specific global evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunCleanupWakeProgress {
    /// Eligibility, execution quiescence or cleanup completion remains uncertain.
    Held,
    /// Qualified cleanup completed; this is neither Drain proof nor a receipt.
    Complete,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn original() -> RunCleanupWakeOriginal {
        RunCleanupWakeOriginal {
            drain_attempt: Uuid::new_v4(),
            drain_command: Uuid::new_v4(),
            removal_id: Uuid::new_v4(),
            command_key: [1; 32],
            input_hash: [2; 32],
            expected_version: 1,
            creator: Uuid::new_v4(),
            request: Uuid::new_v4(),
        }
    }
    #[test]
    fn original_identifiers_and_cas_are_checked_without_owner_or_profile_input() {
        let mut pins = original();
        pins.creator = Uuid::nil();
        assert!(RunCleanupWakeRequest::new(RunId::new(), AgentInstanceId::new(), pins).is_err());
        let mut pins = original();
        pins.expected_version = 0;
        assert!(RunCleanupWakeRequest::new(RunId::new(), AgentInstanceId::new(), pins).is_err());
        let pins = original();
        let run = RunId::new();
        let instance = AgentInstanceId::new();
        let request = RunCleanupWakeRequest::new(run, instance, pins.clone()).unwrap();
        assert_eq!(request.run_id(), run);
        assert_eq!(request.instance_id(), instance);
        assert_eq!(request.original(), &pins);
    }
}
