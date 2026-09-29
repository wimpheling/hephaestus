use async_trait::async_trait;
use mailbox_domain::{MailboxEventId, MailboxOperationId};
use run_domain::{Run, StartRun};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Minimal durable command payload for a mailbox lifecycle transition.
///
/// The operation ID is deterministic and comes from the domain operation
/// identity. The authoritative store validates its relationship to the event
/// before doing any work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxDispatchCommand {
    /// Stable command operation identity used for idempotency.
    pub operation_id: MailboxOperationId,
    /// Accepted event being woken, dispatched, retried, cancelled, or recovered.
    #[serde(rename = "mailbox_event_id", alias = "event_id")]
    pub event_id: MailboxEventId,
}

/// A committed command record ready for `JetStream` publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxOutboxRecord {
    /// Stable transactional-outbox record ID, used as `Nats-Msg-Id`.
    pub id: Uuid,
    /// Opaque ownership token required to settle this short-lived claim.
    pub claim_token: Uuid,
    /// One exact versioned command subject.
    pub subject: String,
    /// Identifier-only command body.
    pub command: MailboxDispatchCommand,
}

/// PostgreSQL boundary for mailbox command dispatch and result settlement.
///
/// Implementations must use compare-and-swap delivery transitions. Returning
/// `None` from [`claim_dispatch`](Self::claim_dispatch) proves that this
/// command has already been completed, cancelled, denied, or is otherwise not
/// eligible, and is therefore safe to acknowledge at the transport layer.
#[async_trait]
pub trait MailboxDispatchStore: Send + Sync + 'static {
    /// Claims committed mailbox outbox commands for publication.
    async fn claim_outbox(
        &self,
        subjects: &[&str],
        limit: i64,
    ) -> Result<Vec<MailboxOutboxRecord>, MailboxDispatchStoreError>;
    /// Marks a broker-confirmed command as published.
    async fn mark_outbox_published(
        &self,
        id: Uuid,
        claim_token: Uuid,
    ) -> Result<(), MailboxDispatchStoreError>;
    /// Records a broker publication failure without changing logical delivery state.
    async fn mark_outbox_failed(
        &self,
        id: Uuid,
        claim_token: Uuid,
        error: &str,
    ) -> Result<(), MailboxDispatchStoreError>;
    /// Applies a wake, retry, cancellation, or recovery operation idempotently.
    async fn apply_command(
        &self,
        subject: &str,
        command: &MailboxDispatchCommand,
    ) -> Result<(), MailboxDispatchStoreError>;
    /// Compare-and-swap claims the exact event for execution and creates the
    /// corresponding durable run request. It returns a run command only once.
    async fn claim_dispatch(
        &self,
        command: &MailboxDispatchCommand,
    ) -> Result<Option<StartRun>, MailboxDispatchStoreError>;
    /// Copies the run's exact fenced resource evidence to its delivery attempt
    /// before guest provisioning. Implementations must make this idempotent.
    async fn record_run_resources(&self, run: &Run) -> Result<(), MailboxDispatchStoreError>;
    /// Settles the mailbox attempt only after VM destruction, lease release,
    /// and the run's durable `CleanedUp` transition.
    async fn settle_run(&self, run: &Run) -> Result<(), MailboxDispatchStoreError>;
    /// Reconciles interrupted mailbox claims and outcome settlements.
    async fn recover(&self) -> Result<usize, MailboxDispatchStoreError>;
    /// Purges expired opaque body bytes only after durable terminal delivery.
    /// Immutable event and integrity provenance remain available afterwards.
    async fn cleanup_expired_payloads(
        &self,
        limit: i64,
    ) -> Result<usize, MailboxDispatchStoreError>;
}

/// Non-disclosing failure returned by the mailbox authoritative store.
#[derive(Debug, thiserror::Error)]
#[error("mailbox dispatch store failed: {0}")]
pub struct MailboxDispatchStoreError(pub String);
