use release_domain::{AgentInstanceId, AgentInstanceRevisionId};
use runtime_types::{CommandId, RunId};

use super::{InstanceActivationId, InstanceExecutionError, InstanceInvocationId, ids::require_ids};

/// Checked activation receipt data, separate from physical readiness evidence.
///
/// Construction validates data only. The service must prove committed lineage,
/// current authority and the current open gate before returning this admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceActivationAdmission {
    activation_id: InstanceActivationId,
    instance_id: AgentInstanceId,
    revision_id: AgentInstanceRevisionId,
    activated_version: u64,
}

impl InstanceActivationAdmission {
    /// Checks the first profile's immutable activation result data.
    ///
    /// # Errors
    /// Rejects nil instance/revision IDs and versions other than 2.
    pub const fn new(
        activation_id: InstanceActivationId,
        instance_id: AgentInstanceId,
        revision_id: AgentInstanceRevisionId,
        activated_version: u64,
    ) -> Result<Self, InstanceExecutionError> {
        if let Err(error) = require_ids(&[instance_id.as_uuid(), revision_id.as_uuid()]) {
            return Err(error);
        }
        if activated_version != 2 {
            return Err(InstanceExecutionError::InvalidVersion);
        }
        Ok(Self {
            activation_id,
            instance_id,
            revision_id,
            activated_version,
        })
    }

    /// Returns the original activation identity.
    #[must_use]
    pub const fn activation_id(&self) -> InstanceActivationId {
        self.activation_id
    }
    /// Returns the activated consumer.
    #[must_use]
    pub const fn instance_id(&self) -> AgentInstanceId {
        self.instance_id
    }
    /// Returns the exact immutable revision.
    #[must_use]
    pub const fn revision_id(&self) -> AgentInstanceRevisionId {
        self.revision_id
    }
    /// Returns the one-time committed activation version, 2.
    #[must_use]
    pub const fn activated_version(&self) -> u64 {
        self.activated_version
    }
}

/// Checked Invocation admission data, without runtime execution proof.
///
/// A service returns the immutable request/Run/command correspondence only after
/// its authoritative transaction commits. This DTO proves no provider outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceInvocationAdmission {
    invocation: InstanceInvocationId,
    instance: AgentInstanceId,
    revision: AgentInstanceRevisionId,
    run: RunId,
    start_command: CommandId,
}

impl InstanceInvocationAdmission {
    /// Checks a complete immutable Invocation result correspondence.
    ///
    /// # Errors
    /// Rejects nil instance, revision, Run or Start command IDs.
    pub const fn new(
        invocation_id: InstanceInvocationId,
        instance_id: AgentInstanceId,
        revision_id: AgentInstanceRevisionId,
        run_id: RunId,
        start_command_id: CommandId,
    ) -> Result<Self, InstanceExecutionError> {
        if let Err(error) = require_ids(&[
            instance_id.as_uuid(),
            revision_id.as_uuid(),
            run_id.as_uuid(),
            start_command_id.as_uuid(),
        ]) {
            return Err(error);
        }
        Ok(Self {
            invocation: invocation_id,
            instance: instance_id,
            revision: revision_id,
            run: run_id,
            start_command: start_command_id,
        })
    }

    /// Returns the original Invocation identity.
    #[must_use]
    pub const fn invocation_id(&self) -> InstanceInvocationId {
        self.invocation
    }
    /// Returns the admitted consumer.
    #[must_use]
    pub const fn instance_id(&self) -> AgentInstanceId {
        self.instance
    }
    /// Returns its immutable revision.
    #[must_use]
    pub const fn revision_id(&self) -> AgentInstanceRevisionId {
        self.revision
    }
    /// Returns the original execution identity.
    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run
    }
    /// Returns the original durable Start command identity.
    #[must_use]
    pub const fn start_command_id(&self) -> CommandId {
        self.start_command
    }
}
