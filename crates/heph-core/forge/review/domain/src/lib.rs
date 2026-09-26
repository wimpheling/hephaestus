//! Provider-neutral durable commands for human review and run control.

use forge_domain::RepositoryId;
use identity_domain::{RequestId, UserId};
use runtime_types::RunId;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use uuid::Uuid;

/// Durable NATS subject carrying committed browser control intents.
pub const CONTROL_EXECUTE_SUBJECT: &str = "hephaestus.control.execute";

macro_rules! identifier {
    ($name:ident, $documentation:literal) => {
        #[doc = $documentation]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Creates a new opaque identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Reconstructs an identifier from durable storage.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the durable UUID.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self)
            }
        }
    };
}

identifier!(
    ReviewProposalId,
    "Opaque identifier for one controlled result proposal."
);
identifier!(
    ControlRequestId,
    "Idempotency key for one browser-originated control request."
);

/// Supported durable human control operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    /// Request cancellation of an active run.
    CancelRun,
    /// Create a new run from the exact same accepted input.
    RetryRun,
    /// Fast-forward the proposal target ref using compare-and-swap.
    ApproveResult,
    /// Close a proposal without changing Git.
    RejectResult,
}

/// Command published from a committed [`ControlRequestId`] outbox record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlCommand {
    /// Durable command and control-request identifier.
    pub command_id: ControlRequestId,
    /// Requested operation.
    pub kind: ControlKind,
    /// Authenticated internal actor.
    pub actor_id: UserId,
    /// Browser request correlation identifier.
    pub request_id: RequestId,
    /// Repository perimeter for the operation.
    pub repository_id: RepositoryId,
    /// Target run for cancellation or retry.
    pub run_id: Option<RunId>,
    /// Target review proposal for approval or rejection.
    pub proposal_id: Option<ReviewProposalId>,
    /// Optional human explanation.
    pub reason: String,
}

impl ControlCommand {
    /// Validates that the target shape matches the command kind.
    ///
    /// # Errors
    ///
    /// Returns an error when a run command names a proposal or a review
    /// command names a run.
    pub const fn validate(&self) -> Result<(), InvalidControlCommand> {
        let valid = match self.kind {
            ControlKind::CancelRun | ControlKind::RetryRun => {
                self.run_id.is_some() && self.proposal_id.is_none()
            }
            ControlKind::ApproveResult | ControlKind::RejectResult => {
                self.run_id.is_none() && self.proposal_id.is_some()
            }
        };
        if valid {
            Ok(())
        } else {
            Err(InvalidControlCommand)
        }
    }
}

/// A command carried targets that do not match its operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("control command target does not match its operation")]
pub struct InvalidControlCommand;

#[cfg(test)]
mod tests {
    use super::{ControlCommand, ControlKind, ControlRequestId, ReviewProposalId};
    use forge_domain::RepositoryId;
    use identity_domain::{RequestId, UserId};
    use runtime_types::RunId;

    #[test]
    fn validates_operation_target_shapes() {
        let mut command = ControlCommand {
            command_id: ControlRequestId::new(),
            kind: ControlKind::CancelRun,
            actor_id: UserId::new(),
            request_id: RequestId::new(),
            repository_id: RepositoryId::new(),
            run_id: Some(RunId::new()),
            proposal_id: None,
            reason: String::new(),
        };
        assert!(command.validate().is_ok());
        command.proposal_id = Some(ReviewProposalId::new());
        assert!(command.validate().is_err());
    }
}
