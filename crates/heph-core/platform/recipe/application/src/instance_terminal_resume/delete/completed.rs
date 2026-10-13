use super::validate_original;
use crate::{
    DeploymentCommandId, DeploymentError, PreparedInstanceDeleteContinuation,
    PreparedInstanceDeletePublication, PreparedInstanceTerminalProgress,
};

/// Closed original terminal plus unique actual Applied endpoint comparison.
///
/// A protected adapter must positively qualify actual transition/worker proof,
/// event/fingerprint, completed Detach and whole original history. Construction
/// compares their projected pins only: it observes no database or physical state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceDeleteCompletedHistory {
    original: PreparedInstanceDeleteContinuation,
    observed: PreparedInstanceTerminalProgress,
    publisher_admission_command: Option<DeploymentCommandId>,
}
impl PreparedInstanceDeleteCompletedHistory {
    /// Requires committed original metadata and the exact cleared Applied endpoint.
    ///
    /// # Errors
    /// Rejects unwritten/null metadata, active or non-Deleted progress, wrong prior
    /// action/generation, changed publisher admission and invalid original before CAS.
    pub fn new(
        original: PreparedInstanceDeleteContinuation,
        observed: PreparedInstanceTerminalProgress,
        publisher_admission_command: Option<DeploymentCommandId>,
    ) -> Result<Self, DeploymentError> {
        validate_original(&original, publisher_admission_command)?;
        if !matches!(
            original.publication(),
            PreparedInstanceDeletePublication::Committed(_)
        ) {
            return Err(DeploymentError::ReconciliationRequired);
        }
        observed.applied(original.original().effect())?;
        Ok(Self {
            original,
            observed,
            publisher_admission_command,
        })
    }
    pub(super) fn into_parts(
        self,
    ) -> (
        PreparedInstanceDeleteContinuation,
        PreparedInstanceTerminalProgress,
        Option<DeploymentCommandId>,
    ) {
        (
            self.original,
            self.observed,
            self.publisher_admission_command,
        )
    }
    /// Returns full original before CAS and separately saved publisher data.
    #[must_use]
    pub const fn original(&self) -> &PreparedInstanceDeleteContinuation {
        &self.original
    }
    /// Returns all seven projected fields from actual qualified Applied history.
    #[must_use]
    pub const fn observed_progress(&self) -> &PreparedInstanceTerminalProgress {
        &self.observed
    }
    /// Returns the original publisher's admission link, never an authorization.
    #[must_use]
    pub const fn publisher_admission_command(&self) -> Option<DeploymentCommandId> {
        self.publisher_admission_command
    }
}
