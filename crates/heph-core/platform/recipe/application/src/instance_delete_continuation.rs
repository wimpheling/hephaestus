//! Durable Delete comparison data, never admission, absence or physical proof.
mod publication;
use crate::{CommandIdentity, DeploymentError, PreparedInstanceDeleteRequest};
pub use publication::PreparedInstanceDeletePublicationRecord;

/// Explicit observation kind; no missing-row or unknown-state fallback exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparedInstanceDeletePublication {
    /// Native reader must positively qualify an unwritten original active Delete.
    /// This pure tag alone does not prove absence or permit a write.
    Unwritten,
    /// Exact committed original; boxed to keep the closed variant bounded.
    Committed(Box<PreparedInstanceDeletePublicationRecord>),
}

/// Full original Delete inputs with authoritative original before-instance CAS.
///
/// Native143 must independently qualify original Create, tagged closure, committed
/// Detach, full cleanup history/config and current admission. The saved publisher
/// is comparison data separate from the current cleanup command, never credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceDeleteContinuation {
    original: PreparedInstanceDeleteRequest,
    publication: PreparedInstanceDeletePublication,
}
impl PreparedInstanceDeleteContinuation {
    /// Retains a checked unwritten input, without inferring absence from a lookup.
    ///
    /// Before CAS comes from the protected native original observation. No resource
    /// version or current tombstone is substituted. A bigint maximum is representable
    /// comparison data but cannot produce a committed successor.
    #[must_use]
    pub const fn unwritten(original: PreparedInstanceDeleteRequest) -> Self {
        Self {
            original,
            publication: PreparedInstanceDeletePublication::Unwritten,
        }
    }
    /// Compares full expected originals with actual recorded terminal metadata.
    ///
    /// Only current cleanup command may differ from the saved publisher's command.
    /// Neither identity is authenticated here; a native adapter rechecks current
    /// ALL-owned rights before any mutation/completion. No Detach proof is accepted.
    ///
    /// # Errors
    /// Rejects changed whole intent/Create/target/tagged closure/Delete claim,
    /// generation/configuration or original before-instance CAS.
    pub fn committed(
        original: PreparedInstanceDeleteRequest,
        recorded: PreparedInstanceDeletePublicationRecord,
    ) -> Result<Self, DeploymentError> {
        if !recorded.matches(&original) {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(Self {
            original,
            publication: PreparedInstanceDeletePublication::Committed(Box::new(recorded)),
        })
    }
    /// Returns every checked original pin, including its stored before-instance CAS.
    #[must_use]
    pub const fn original(&self) -> &PreparedInstanceDeleteRequest {
        &self.original
    }
    /// Returns explicit recorded/unwritten comparison state, not execution authority.
    #[must_use]
    pub const fn publication(&self) -> &PreparedInstanceDeletePublication {
        &self.publication
    }
    /// Rebuilds comparison input for a separately authenticated current Remove.
    ///
    /// Preserves original before CAS, full Delete claim and recorded publisher; never
    /// guesses from current Removed version or resource version. Native143 must still
    /// qualify actual current admission/rights and complete original history.
    ///
    /// # Errors
    /// Rejects non-Remove current command or any invalid existing checked request pins.
    pub fn rebuild_for_remove(
        &self,
        current: CommandIdentity,
    ) -> Result<PreparedInstanceDeleteRequest, DeploymentError> {
        PreparedInstanceDeleteRequest::new(
            self.original.intent().clone(),
            self.original.target().clone(),
            self.original.closure().clone(),
            self.original.effect().clone(),
            current,
            self.original.configuration().clone(),
            self.original.expected_instance_version(),
        )
    }
}
