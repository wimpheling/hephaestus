//! Qualified queued preparation is metadata, never a physical cleanup receipt.
use run_domain::{Run, RunCleanupTarget};

/// Positively observed immutable Invocation evidence for cancellation dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationCancelEvidence {
    /// Genuine115 original without117 consumption, including prepared cleanup replay.
    Queued115,
    /// Actual immutable117 consumption and processed original Start match.
    Consumed117,
}

/// Strict prepared queued cancellation and its exact immutable cleanup target.
#[derive(Debug, Clone)]
pub struct PreparedInvocationCancellation {
    /// The Run hydrated only after protected preparation and strict qualification.
    pub run: Run,
    /// Complete persisted target; a zero set is not physical absence proof.
    pub target: RunCleanupTarget,
}

/// Result of the protected preparation port, independently requalified by it.
#[derive(Debug, Clone)]
pub enum InvocationCancelPreparation {
    /// Positive consumed117 history; the existing Start owns active cleanup.
    Consumed117,
    /// Processed genuine cancellation with strictly prepared queued history.
    PreparedQueued(Box<PreparedInvocationCancellation>),
}
