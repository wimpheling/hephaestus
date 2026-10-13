pub(super) mod completed;
use super::PreparedInstanceTerminalProgress;
use crate::{
    DeploymentCommandId, DeploymentError, PreparedInstanceDeleteContinuation,
    PreparedInstanceDeletePublication,
};
use completed::PreparedInstanceDeleteCompletedHistory;

/// Closed comparison state; neither variant is dispatch or physical authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparedInstanceDeleteHistoryState {
    /// Exact original active attempt, possibly with an already committed tombstone.
    Active,
    /// Positively qualified committed terminal and unique actual Applied history.
    Applied,
}

/// Native terminal7 comparison data preserving the accepted `CoreDelete6` originals.
///
/// Full configuration/Create/tagged closure/claim/original before CAS remain in
/// the retained continuation. The seven-field endpoint is separately retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceDeleteHistory {
    original: PreparedInstanceDeleteContinuation,
    observed: PreparedInstanceTerminalProgress,
    publisher_admission_command: Option<DeploymentCommandId>,
    state: PreparedInstanceDeleteHistoryState,
}
impl PreparedInstanceDeleteHistory {
    /// Checks exact active original progress and the whole committed/unwritten tuple.
    ///
    /// No missing row is interpreted here; an unwritten historical endpoint remains
    /// comparison data even after publication. Adapter observation must be positive.
    ///
    /// # Errors
    /// Rejects substituted attempt/generation/phase, before-CAS at bigint maximum,
    /// unknown publisher admission or conflicting committed metadata.
    pub fn active(
        original: PreparedInstanceDeleteContinuation,
        observed: PreparedInstanceTerminalProgress,
        publisher_admission_command: Option<DeploymentCommandId>,
    ) -> Result<Self, DeploymentError> {
        validate_original(&original, publisher_admission_command)?;
        observed.active(original.original().effect())?;
        Ok(Self {
            original,
            observed,
            publisher_admission_command,
            state: PreparedInstanceDeleteHistoryState::Active,
        })
    }
    /// Retains a separately checked completed observation; inactive is never inferred.
    #[must_use]
    pub fn completed(completed: PreparedInstanceDeleteCompletedHistory) -> Self {
        let (original, observed, publisher_admission_command) = completed.into_parts();
        Self {
            original,
            observed,
            publisher_admission_command,
            state: PreparedInstanceDeleteHistoryState::Applied,
        }
    }
    /// Checks the atomic nullable native terminal tuple using accepted `CoreDelete6`.
    ///
    /// All absent fields require a positively observed active endpoint; all present
    /// fields retain the actual record and exact before+1. Partial tuples deny.
    /// No absent lookup is interpreted as native qualification.
    ///
    /// # Errors
    /// Rejects partial/null tuples, changed publisher/claim/CAS, active-state drift
    /// or an inactive endpoint lacking committed actual Applied history.
    pub fn from_observed_parts(
        request: crate::PreparedInstanceDeleteRequest,
        observed: PreparedInstanceTerminalProgress,
        operation: Option<uuid::Uuid>,
        after: Option<u64>,
        publisher: Option<super::PreparedInstanceTerminalPublisher>,
    ) -> Result<Self, DeploymentError> {
        match (operation, after, publisher) {
            (None, None, None) => Self::active(
                PreparedInstanceDeleteContinuation::unwritten(request),
                observed,
                None,
            ),
            (Some(operation), Some(after), Some(publisher)) => {
                let recorded = crate::PreparedInstanceDeleteRequest::new(
                    request.intent().clone(),
                    request.target().clone(),
                    request.closure().clone(),
                    request.effect().clone(),
                    publisher.command(),
                    request.configuration().clone(),
                    request.expected_instance_version(),
                )?;
                let record = crate::PreparedInstanceDeletePublicationRecord::new(
                    crate::PreparedInstanceDeleteObservation::new(recorded, operation, after)?,
                    publisher.command(),
                    publisher.command_id(),
                    publisher.request_id(),
                )?;
                let original = PreparedInstanceDeleteContinuation::committed(request, record)?;
                if observed.progress().active_attempt.is_some() {
                    Self::active(original, observed, publisher.admission_command_id())
                } else {
                    PreparedInstanceDeleteCompletedHistory::new(
                        original,
                        observed,
                        publisher.admission_command_id(),
                    )
                    .map(Self::completed)
                }
            }
            _ => Err(DeploymentError::IntentMismatch),
        }
    }
    /// Returns every accepted original request pin and authoritative before CAS.
    #[must_use]
    pub const fn original(&self) -> &PreparedInstanceDeleteContinuation {
        &self.original
    }
    /// Returns all seven native State fields unchanged.
    #[must_use]
    pub const fn observed_progress(&self) -> &PreparedInstanceTerminalProgress {
        &self.observed
    }
    /// Returns saved actual admission-command link; never a current credential.
    #[must_use]
    pub const fn publisher_admission_command(&self) -> Option<DeploymentCommandId> {
        self.publisher_admission_command
    }
    /// Returns exact checked comparison state, not permission or receipt.
    #[must_use]
    pub const fn state(&self) -> PreparedInstanceDeleteHistoryState {
        self.state
    }
}
fn validate_original(
    original: &PreparedInstanceDeleteContinuation,
    admission: Option<DeploymentCommandId>,
) -> Result<(), DeploymentError> {
    let request = original.original();
    if request.expected_instance_version() >= i64::MAX.cast_unsigned()
        || request.closure().drain.generation.checked_add(2) != Some(request.effect().generation)
        || request
            .closure()
            .drain
            .resource_version
            .checked_add(4)
            .is_none_or(|version| request.effect().resource_version < version)
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let expected = match original.publication() {
        PreparedInstanceDeletePublication::Unwritten => None,
        PreparedInstanceDeletePublication::Committed(record) => {
            (record.publisher() != request.effect().command).then_some(record.native_command_id())
        }
    };
    if admission != expected {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(())
}
