use crate::receive_policy::ReceivePolicyError;
use std::io;

/// Fail-closed pre-receive inspection error.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReceiveHookError {
    /// Host authority does not match the selected repository or current time.
    #[error("runtime receive context does not match this transaction")]
    ContextMismatch,
    /// The hook command stream was empty, malformed, or over its bound.
    #[error("runtime receive command batch is invalid")]
    InvalidCommandBatch,
    /// A client-declared old object did not match the canonical ref.
    #[error("runtime receive command is stale or inconsistent with canonical state")]
    StaleOrForgedCommand,
    /// Canonical bare storage was not trustworthy.
    #[error("runtime receive repository layout is invalid")]
    InvalidRepositoryLayout,
    /// Git's quarantine was absent or outside canonical object storage.
    #[error("runtime receive quarantine is invalid")]
    InvalidQuarantine,
    /// Trusted object or ancestry facts could not be derived.
    #[error("runtime receive repository facts are invalid")]
    InvalidRepositoryFacts,
    /// A bounded Git inspection subprocess failed.
    #[error("runtime receive repository inspection failed")]
    GitCommand,
    /// Inspection output exceeded its host-side safety ceiling.
    #[error("runtime receive inspection output exceeded its safety limit")]
    InspectionOutputLimitExceeded,
    /// Object or byte ceilings were exceeded.
    #[error("runtime receive transfer limit was exceeded")]
    TransferLimitExceeded,
    /// Capability policy denied the trusted proposal.
    #[error(transparent)]
    Policy(#[from] ReceivePolicyError),
    /// Local storage inspection failed.
    #[error("runtime receive local inspection failed")]
    Io(#[source] io::Error),
}
