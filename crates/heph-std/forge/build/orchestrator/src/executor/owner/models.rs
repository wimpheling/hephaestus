use super::super::{BuildExecutionError, BuildExecutionResult, BuildRequestId};
use std::sync::Arc;

/// Comparison-only state of one original live build owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildCleanupPhase {
    /// An actual owning execution/preparation job is still unsettled.
    Executing,
    /// Original VM detach is unresolved.
    VmHeld,
    /// Original workspace identity/removal is unresolved.
    WorkspaceHeld,
    /// Sealed/imported original output needs genuine finalization.
    FinalizationRequired,
    /// Jobs settled, original VM detached, workspace cleanup confirmed.
    Settled,
}
/// Nonsecret original cleanup observation; never physical authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildCleanupStatus {
    /// Original durable request.
    pub build_request_id: BuildRequestId,
    /// Observed phase.
    pub phase: BuildCleanupPhase,
    /// Number of retained original errors (not raw diagnostics).
    pub retained_errors: usize,
}
/// Bounded observation failed; actual jobs/custody remain with the executor.
#[derive(Debug, thiserror::Error)]
#[error("original build cleanup remains held")]
pub struct BuildCleanupError {
    /// Original unsettled observations. Empty never implies hostwide absence.
    pub remaining: Vec<BuildCleanupStatus>,
    /// Original configured workspace-root identity could not be revalidated.
    pub root_held: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Execute,
    Retry,
    Verify,
    Finalize,
}
#[derive(Clone)]
pub(super) enum Outcome {
    Build(Result<BuildExecutionResult, Arc<BuildExecutionError>>),
    Verify(Result<(), Arc<BuildExecutionError>>),
}
impl Outcome {
    pub(super) const fn failed(&self) -> bool {
        match self {
            Self::Build(result) => result.is_err(),
            Self::Verify(result) => result.is_err(),
        }
    }
}
