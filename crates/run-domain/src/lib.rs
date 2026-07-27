//! Durable run state, outcomes, commands, and transition rules.

use runtime_types::{AgentId, CommandId, LeaseId, RunId, VolumeId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use vm_trait::VmExit;

/// Durable lifecycle state of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RunState {
    /// Accepted and waiting for orchestration.
    Queued,
    /// Acquiring the exclusive agent-state lease.
    LeasingVolume,
    /// Allocating provider-owned VM resources.
    Provisioning,
    /// Booting the guest and waiting for readiness.
    Starting,
    /// The guest workload is executing.
    Running,
    /// Workload execution completed successfully.
    Succeeded,
    /// Workload execution or orchestration failed.
    Failed,
    /// Cancellation was accepted.
    Cancelled,
    /// VM and volume resources are being released.
    CleaningUp,
    /// All transient resources and the lease were released.
    CleanedUp,
}

impl RunState {
    /// Returns whether moving from this state to `next` is valid.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        use RunState::{
            Cancelled, CleanedUp, CleaningUp, Failed, LeasingVolume, Provisioning, Queued, Running,
            Starting, Succeeded,
        };
        matches!(
            (self, next),
            (Queued, LeasingVolume | Cancelled | Failed)
                | (LeasingVolume, Provisioning | Cancelled | Failed)
                | (Provisioning, Starting | Cancelled | Failed)
                | (Starting, Running | Cancelled | Failed)
                | (Running, Succeeded | Cancelled | Failed)
                | (Succeeded | Failed | Cancelled, CleaningUp)
                | (CleaningUp, CleanedUp)
        )
    }

    /// Returns the final execution outcome represented by this state.
    #[must_use]
    pub const fn outcome(self) -> Option<RunOutcome> {
        match self {
            Self::Succeeded => Some(RunOutcome::Succeeded),
            Self::Failed => Some(RunOutcome::Failed),
            Self::Cancelled => Some(RunOutcome::Cancelled),
            _ => None,
        }
    }
}

/// Execution result retained after cleanup completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RunOutcome {
    /// The workload exited successfully.
    Succeeded,
    /// Execution or orchestration failed.
    Failed,
    /// A cancellation command ended the run.
    Cancelled,
}

/// Durable representation of one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// Stable run identifier.
    pub id: RunId,
    /// Agent being executed.
    pub agent_id: AgentId,
    /// Command that created this run.
    pub command_id: CommandId,
    /// Persistent state volume, once resolved.
    pub volume_id: Option<VolumeId>,
    /// Writable lease, once acquired.
    pub lease_id: Option<LeaseId>,
    /// Provider-neutral VM identifier.
    pub vm_id: Option<String>,
    /// Current durable lifecycle state.
    pub state: RunState,
    /// Execution result retained through cleanup.
    pub outcome: Option<RunOutcome>,
    /// Final VM exit, when available.
    pub exit: Option<VmExit>,
    /// Durable failure diagnostic.
    pub failure: Option<String>,
    /// Time at which cancellation was requested.
    pub cancel_requested_at: Option<OffsetDateTime>,
    /// Creation time.
    pub created_at: OffsetDateTime,
    /// Most recent state change.
    pub updated_at: OffsetDateTime,
    /// Optimistic state-machine version.
    pub state_version: i64,
}

/// Idempotent request to start an agent run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartRun {
    /// Idempotency key for duplicate delivery handling.
    pub command_id: CommandId,
    /// Stable run identifier selected by the command producer.
    pub run_id: RunId,
    /// Agent to execute.
    pub agent_id: AgentId,
}

/// Idempotent request to cancel a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelRun {
    /// Idempotency key for duplicate delivery handling.
    pub command_id: CommandId,
    /// Run to cancel.
    pub run_id: RunId,
    /// Human-readable cancellation reason.
    pub reason: String,
}

/// Failure to apply a run-domain transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidTransition {
    /// State before the attempted transition.
    pub current: RunState,
    /// Rejected next state.
    pub requested: RunState,
}

impl std::fmt::Display for InvalidTransition {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "cannot transition a run from {:?} to {:?}",
            self.current, self.requested
        )
    }
}

impl std::error::Error for InvalidTransition {}

#[cfg(test)]
mod tests {
    use super::{RunOutcome, RunState};

    #[test]
    fn successful_result_survives_cleanup_states() {
        assert!(RunState::Running.can_transition_to(RunState::Succeeded));
        assert!(RunState::Succeeded.can_transition_to(RunState::CleaningUp));
        assert!(RunState::CleaningUp.can_transition_to(RunState::CleanedUp));
        assert_eq!(RunState::Succeeded.outcome(), Some(RunOutcome::Succeeded));
        assert_eq!(RunState::CleanedUp.outcome(), None);
    }

    #[test]
    fn cancellation_is_valid_before_and_during_execution() {
        for state in [
            RunState::Queued,
            RunState::LeasingVolume,
            RunState::Provisioning,
            RunState::Starting,
            RunState::Running,
        ] {
            assert!(state.can_transition_to(RunState::Cancelled));
        }
    }

    #[test]
    fn cleanup_cannot_skip_execution_outcome() {
        assert!(!RunState::Running.can_transition_to(RunState::CleaningUp));
        assert!(!RunState::CleanedUp.can_transition_to(RunState::Running));
    }
}
