use crate::{
    CommandIdentity, DeploymentCommandId, DeploymentError, DeploymentOperation, EffectClaim,
};
use identity_domain::{RequestId, UserId};

/// Exact four-field terminal publisher DTO joined to its original logical command.
///
/// The saved actor/request are data, independent of the current manager. Adapters
/// must qualify the original command row and admission; IDs never rebuild authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceTerminalPublisher {
    command: CommandIdentity,
    command_id: DeploymentCommandId,
    actor_id: UserId,
    request_id: RequestId,
    admission_command_id: Option<DeploymentCommandId>,
}
impl PreparedInstanceTerminalPublisher {
    /// Compares the original logical command, actual actor/request and nullable link.
    ///
    /// # Errors
    /// Rejects non-Remove/wrong ID/actor/nil request and any admission link other
    /// than None for original Delete publisher or that exact fresh publisher ID.
    pub fn new(
        effect: &EffectClaim,
        command: CommandIdentity,
        command_id: DeploymentCommandId,
        actor_id: UserId,
        request_id: RequestId,
        admission_command_id: Option<DeploymentCommandId>,
    ) -> Result<Self, DeploymentError> {
        let expected = (command != effect.command).then_some(command_id);
        if effect.action != crate::ResourceAction::Delete
            || command.operation() != DeploymentOperation::Remove
            || command_id != command.id()
            || actor_id != command.actor_id()
            || request_id.as_uuid().is_nil()
            || admission_command_id != expected
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(Self {
            command,
            command_id,
            actor_id,
            request_id,
            admission_command_id,
        })
    }
    /// Returns recorded original command, not the current caller's credentials.
    #[must_use]
    pub const fn command(&self) -> CommandIdentity {
        self.command
    }
    /// Returns exact native publisher command ID.
    #[must_use]
    pub const fn command_id(&self) -> DeploymentCommandId {
        self.command_id
    }
    /// Returns recorded publisher actor, not an identity to authenticate.
    #[must_use]
    pub const fn actor_id(&self) -> UserId {
        self.actor_id
    }
    /// Returns recorded publication request independently of current request.
    #[must_use]
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }
    /// Returns the exact nullable native resume-admission link.
    #[must_use]
    pub const fn admission_command_id(&self) -> Option<DeploymentCommandId> {
        self.admission_command_id
    }
}
