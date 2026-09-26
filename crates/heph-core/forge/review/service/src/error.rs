use review_domain::{ControlRequestId, ReviewProposalId};
use runtime_types::RunId;

/// Provider-neutral persistence failure for review units of work.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReviewRepositoryError {
    /// A delivery did not match its authoritative persisted command.
    #[error("control delivery does not match its authoritative row")]
    DeliveryMismatch,
    /// The durable control request does not exist.
    #[error("control request {0} does not exist")]
    MissingControl(ControlRequestId),
    /// The review proposal does not exist.
    #[error("review proposal {0} does not exist")]
    MissingProposal(ReviewProposalId),
    /// The source run did not originate from an accepted forge request.
    #[error("run {0} has no forge run request")]
    MissingRunRequest(RunId),
    /// The proposal is no longer actionable.
    #[error("review proposal is closed in state {0}")]
    ProposalClosed(String),
    /// A persistence or authorization provider failed.
    #[error("review repository operation failed: {0}")]
    Infrastructure(String),
}

/// Durable control processing failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ControlServiceError {
    /// Command targets did not match its operation.
    #[error(transparent)]
    InvalidCommand(#[from] review_domain::InvalidControlCommand),
    /// Persistence or authorization failed.
    #[error(transparent)]
    Repository(#[from] ReviewRepositoryError),
    /// Recorded result provenance failed trusted host validation.
    #[error("invalid result provenance: {0}")]
    InvalidResultProvenance(String),
    /// A Git value stored in persistence was invalid.
    #[error(transparent)]
    GitValue(#[from] forge_domain::GitValueError),
    /// Canonical repository resolution failed.
    #[error("canonical repository resolution failed: {0}")]
    Storage(String),
    /// Git process launch failed.
    #[error("Git process failed: {0}")]
    Io(#[source] std::io::Error),
    /// Git rejected an operation.
    #[error("Git operation failed: {0}")]
    Git(String),
}

/// Control delivery failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ControlHandlingError {
    /// Delivery used an unsupported subject.
    #[error("unsupported control subject {0}")]
    UnknownSubject(String),
    /// Delivery payload was not a valid command.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// Durable processing failed.
    #[error(transparent)]
    Service(#[from] ControlServiceError),
    /// `JetStream` did not confirm acknowledgement.
    #[error("control acknowledgement failed: {0}")]
    Acknowledgement(String),
}
