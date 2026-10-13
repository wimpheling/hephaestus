use crate::{
    CommandIdentity, DeploymentCommandId, DeploymentError, DeploymentOperation,
    PreparedInstanceDeleteObservation, PreparedInstanceDeleteRequest,
};
use identity_domain::RequestId;
use uuid::Uuid;

/// Closed exact terminal-record comparison fields, not a worker Applied receipt.
///
/// The original checked observation binds full intent/Create/target/tagged closure,
/// Delete claim/config/before CAS and nonnil operation/exact after CAS. Native143
/// qualifies its actual provenance, Detach/history/event and original publisher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceDeletePublicationRecord {
    observation: PreparedInstanceDeleteObservation,
    publisher: CommandIdentity,
    native_command_id: DeploymentCommandId,
    publication_request: RequestId,
}
impl PreparedInstanceDeletePublicationRecord {
    /// Compares actual recorded publisher to the recorded original request.
    ///
    /// The supplied observation is checked comparison data only. Publisher command
    /// must be Remove, match recorded request's command and derive exactly the native
    /// command ID. Publication request is separately retained; it need not equal
    /// the Delete claim's historical request or logical command idempotency key.
    ///
    /// # Errors
    /// Rejects non-Remove/wrong publisher, mismatched native ID or nil publication request.
    pub fn new(
        observation: PreparedInstanceDeleteObservation,
        publisher: CommandIdentity,
        native_command_id: DeploymentCommandId,
        publication_request: RequestId,
    ) -> Result<Self, DeploymentError> {
        if publisher.operation() != DeploymentOperation::Remove
            || observation.request().current_command() != publisher
            || native_command_id != publisher.id()
            || publication_request.as_uuid().is_nil()
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(Self {
            observation,
            publisher,
            native_command_id,
            publication_request,
        })
    }
    pub(super) fn matches(&self, expected: &PreparedInstanceDeleteRequest) -> bool {
        let actual = self.observation.request();
        actual.intent() == expected.intent()
            && actual.target() == expected.target()
            && actual.closure() == expected.closure()
            && actual.effect() == expected.effect()
            && actual.configuration() == expected.configuration()
            && actual.expected_instance_version() == expected.expected_instance_version()
    }
    /// Returns full recorded request/operation/version data, not physical evidence.
    #[must_use]
    pub const fn observation(&self) -> &PreparedInstanceDeleteObservation {
        &self.observation
    }
    /// Returns recorded publisher, independent of current cleanup principal.
    #[must_use]
    pub const fn publisher(&self) -> CommandIdentity {
        self.publisher
    }
    /// Returns actual native command ID compared with typed publisher derivation.
    #[must_use]
    pub const fn native_command_id(&self) -> DeploymentCommandId {
        self.native_command_id
    }
    /// Returns actual recorded publication request, never a credential to reuse.
    #[must_use]
    pub const fn publication_request(&self) -> RequestId {
        self.publication_request
    }
    /// Returns actual nonnil immutable terminal operation identity.
    #[must_use]
    pub const fn terminal_operation_id(&self) -> Uuid {
        self.observation.terminal_operation_id()
    }
    /// Returns original before CAS; current tombstone version is never substituted.
    #[must_use]
    pub const fn before_instance_version(&self) -> u64 {
        self.observation.request().expected_instance_version()
    }
    /// Returns exact original before+1 within signed bigint.
    #[must_use]
    pub const fn after_instance_version(&self) -> u64 {
        self.observation.after_instance_version()
    }
}
