use release_domain::{AgentInstanceId, AgentInstanceRevisionId, ReleaseCommandKey};
use runtime_types::{CommandId, RunId};

use super::{InstanceActivationId, InstanceExecutionError, InstanceInvocationId, ids::require_ids};

/// Backend one-time activation data; fresh authority is separate.
///
/// Private fields preserve checked identifiers and first-profile version bounds.
/// No unchecked deserializer or provider/readiness/actor fields are exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivateInstance {
    command_key: ReleaseCommandKey,
    activation_id: InstanceActivationId,
    instance_id: AgentInstanceId,
    expected_revision_id: AgentInstanceRevisionId,
    expected_creation_version: u64,
}

impl ActivateInstance {
    /// Checks bounded first-profile activation data, without authorizing it.
    ///
    /// # Errors
    /// Rejects nil instance/revision IDs and creation versions other than 1.
    pub const fn new(
        command_key: ReleaseCommandKey,
        activation_id: InstanceActivationId,
        instance_id: AgentInstanceId,
        expected_revision_id: AgentInstanceRevisionId,
        expected_creation_version: u64,
    ) -> Result<Self, InstanceExecutionError> {
        if let Err(error) = require_ids(&[instance_id.as_uuid(), expected_revision_id.as_uuid()]) {
            return Err(error);
        }
        if expected_creation_version != 1 {
            return Err(InstanceExecutionError::InvalidVersion);
        }
        Ok(Self {
            command_key,
            activation_id,
            instance_id,
            expected_revision_id,
            expected_creation_version,
        })
    }

    /// Returns the stable actor-scoped logical command key.
    #[must_use]
    pub const fn command_key(&self) -> ReleaseCommandKey {
        self.command_key
    }
    /// Returns the one-time activation data identity.
    #[must_use]
    pub const fn activation_id(&self) -> InstanceActivationId {
        self.activation_id
    }
    /// Returns the exact immutable consumer.
    #[must_use]
    pub const fn instance_id(&self) -> AgentInstanceId {
        self.instance_id
    }
    /// Returns the exact expected current revision.
    #[must_use]
    pub const fn expected_revision_id(&self) -> AgentInstanceRevisionId {
        self.expected_revision_id
    }
    /// Returns the exact post-import creation version, 1.
    #[must_use]
    pub const fn expected_creation_version(&self) -> u64 {
        self.expected_creation_version
    }
}

/// Backend distinct Invocation data, with no Git or override input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvokeInstance {
    command_key: ReleaseCommandKey,
    invocation_id: InstanceInvocationId,
    run_id: RunId,
    start_command_id: CommandId,
    instance_id: AgentInstanceId,
    expected_revision_id: AgentInstanceRevisionId,
}

impl InvokeInstance {
    /// Checks data identities; this never opens a gate or authorizes execution.
    ///
    /// Run and Start command IDs are predicted by the backend idempotency mapping,
    /// not accepted as authority from clients or inferred from an effect attempt.
    ///
    /// # Errors
    /// Rejects nil Run, Start command, instance or revision IDs.
    pub const fn new(
        command_key: ReleaseCommandKey,
        invocation_id: InstanceInvocationId,
        run_id: RunId,
        start_command_id: CommandId,
        instance_id: AgentInstanceId,
        expected_revision_id: AgentInstanceRevisionId,
    ) -> Result<Self, InstanceExecutionError> {
        if let Err(error) = require_ids(&[
            run_id.as_uuid(),
            start_command_id.as_uuid(),
            instance_id.as_uuid(),
            expected_revision_id.as_uuid(),
        ]) {
            return Err(error);
        }
        Ok(Self {
            command_key,
            invocation_id,
            run_id,
            start_command_id,
            instance_id,
            expected_revision_id,
        })
    }

    /// Returns the stable actor-scoped admission command key.
    #[must_use]
    pub const fn command_key(&self) -> ReleaseCommandKey {
        self.command_key
    }
    /// Returns the immutable Invocation request identity.
    #[must_use]
    pub const fn invocation_id(&self) -> InstanceInvocationId {
        self.invocation_id
    }
    /// Returns the predicted exact execution identity.
    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }
    /// Returns the predicted durable Start command identity.
    #[must_use]
    pub const fn start_command_id(&self) -> CommandId {
        self.start_command_id
    }
    /// Returns the current instance to invoke.
    #[must_use]
    pub const fn instance_id(&self) -> AgentInstanceId {
        self.instance_id
    }
    /// Returns its exact expected immutable revision.
    #[must_use]
    pub const fn expected_revision_id(&self) -> AgentInstanceRevisionId {
        self.expected_revision_id
    }
}
