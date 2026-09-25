use serde::{Deserialize, Serialize};

/// Durable lifecycle of a build request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildState {
    /// Accepted and awaiting execution.
    Queued,
    /// An isolated build guest is executing.
    Running,
    /// Guest stopped successfully and output is being sealed/imported.
    Importing,
    /// Complete durable build output is available.
    Succeeded,
    /// Build or safe import failed.
    Failed,
    /// Authorized cancellation completed.
    Cancelled,
}

impl BuildState {
    /// Returns whether a lifecycle transition is valid.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Queued, Self::Running | Self::Failed | Self::Cancelled)
                | (
                    Self::Running,
                    Self::Importing | Self::Failed | Self::Cancelled
                )
                | (Self::Importing, Self::Succeeded | Self::Failed)
        )
    }
}

/// Durable release lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseState {
    /// Complete build provenance exists but the release is mutable only through
    /// its controlled draft workflow.
    Draft,
    /// Publication permanently froze the release.
    Published,
    /// New use is revoked; historical provenance remains.
    Revoked,
}

impl ReleaseState {
    /// Returns whether a lifecycle transition is valid.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Draft, Self::Published | Self::Revoked) | (Self::Published, Self::Revoked)
        )
    }
}

/// Project-owned instance lifecycle including update recovery states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceState {
    /// Normal runs may be created and dispatched.
    Active,
    /// Instance is disabled by a manager.
    Disabled,
    /// Gate is closed and pre-gate normal runs are draining.
    UpdateDraining,
    /// An isolated update hook owns the state volume.
    Updating,
    /// Candidate was explicitly rejected and the previous revision is safe.
    UpdateRejected,
    /// State compatibility is unknown after abnormal hook failure.
    PausedUnknownState,
    /// Hook committed but candidate activation needs recovery.
    PausedActivationRecovery,
    /// An authorized recovery operation is active.
    Recovering,
    /// Instance is tombstoned; history remains.
    Removed,
}

impl InstanceState {
    /// Returns whether normal requests may bind an active revision.
    #[must_use]
    pub const fn run_gate_open(self) -> bool {
        matches!(self, Self::Active | Self::UpdateRejected)
    }

    /// Returns whether a requested lifecycle transition is valid.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Active,
                Self::Disabled | Self::UpdateDraining | Self::Removed
            ) | (Self::Disabled, Self::Active | Self::Removed)
                | (Self::UpdateDraining, Self::Updating | Self::Active)
                | (
                    Self::Updating | Self::Recovering,
                    Self::Active
                        | Self::UpdateRejected
                        | Self::PausedUnknownState
                        | Self::PausedActivationRecovery
                )
                | (Self::UpdateRejected, Self::Active | Self::UpdateDraining)
                | (
                    Self::PausedUnknownState | Self::PausedActivationRecovery,
                    Self::Recovering
                )
        )
    }
}

/// Candidate update lifecycle and irreversible hook commit point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateState {
    /// Candidate is durable but the run gate is not yet closed.
    Candidate,
    /// Run gate is closed and old work is draining.
    Draining,
    /// Hook is executing in an isolated guest.
    HookRunning,
    /// Hook success is durable and activation must finish.
    HookCommitted,
    /// Candidate revision is active.
    Activated,
    /// Agent explicitly reported safe rollback with a nonzero exit.
    Rejected,
    /// Abnormal failure left state compatibility unknown.
    CompatibilityUnknown,
    /// Activation after committed success requires operator recovery.
    ActivationRecovery,
}

impl UpdateState {
    /// Returns whether a requested lifecycle transition honors the update exit
    /// contract.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Candidate, Self::Draining | Self::Rejected)
                | (Self::Draining, Self::HookRunning | Self::Rejected)
                | (
                    Self::HookRunning,
                    Self::HookCommitted | Self::Rejected | Self::CompatibilityUnknown
                )
                | (
                    Self::HookCommitted,
                    Self::Activated | Self::ActivationRecovery
                )
                | (Self::ActivationRecovery, Self::Activated)
        )
    }
}
