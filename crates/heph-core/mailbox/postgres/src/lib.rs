//! PostgreSQL-backed atomic acceptance of bounded durable mailbox events.
//!
//! The adapter persists opaque body bytes and their envelope in one database
//! transaction. The database trigger then creates the initial delivery row and
//! identifier-only transactional-outbox wake command in that same commit.

mod acceptance;
mod allocation;
mod dispatch_claim;
mod dispatch_commands;
mod dispatch_lifecycle;
mod errors;
mod helpers;
mod operator;
mod recovery;

use mailbox_domain::MailboxEventId;
use sqlx::PgPool;

/// `PostgreSQL` authority for mailbox creation and event acceptance.
#[derive(Clone)]
pub struct PostgresMailboxRepository {
    pub(crate) pool: PgPool,
}

impl PostgresMailboxRepository {
    /// Creates a mailbox adapter over the control-plane pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Explicit operator action retained independently of opaque mailbox bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailboxOperatorAction {
    /// Stop new dispatch claims for a mailbox without dropping accepted work.
    Pause,
    /// Re-open a paused mailbox and wake its deferred deliveries.
    Resume,
    /// Make one retryable or denied delivery eligible immediately.
    Retry,
    /// Prevent a non-terminal delivery from starting again.
    Cancel,
    /// Retain a non-terminal delivery without further automatic execution.
    DeadLetter,
}

impl MailboxOperatorAction {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Retry => "retry",
            Self::Cancel => "cancel",
            Self::DeadLetter => "dead_letter",
        }
    }
}

/// Redacted result of an authorized mailbox control action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailboxOperatorResult {
    /// Whether the requested authoritative state transition occurred.
    pub changed: bool,
}

/// The acceptance result, independent of a transport delivery outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptedMailboxEvent {
    /// Immutable accepted event identity.
    pub event_id: MailboxEventId,
    /// Whether this call observed an already-accepted producer key.
    pub duplicate: bool,
}

pub use errors::MailboxPersistenceError;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
