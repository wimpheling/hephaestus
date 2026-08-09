//! Durable mailbox command transport and VM-completion integration.
//!
//! `PostgreSQL` remains authoritative for every delivery transition. This crate
//! deliberately carries only mailbox-event and stable-operation identifiers in
//! `JetStream`; it never serializes event bodies, capability bearers, or mutable
//! attempt state.

use async_nats::{HeaderMap, jetstream};
use async_trait::async_trait;
use mailbox_domain::{MailboxEventId, MailboxOperationId};
use run_domain::{Run, StartRun};
use run_orchestrator::{
    OrchestratorError, RunCompletionError, RunCompletionObserver, RunOrchestrator,
    RunResourceObservationError, RunResourceObserver,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

/// Wake-up command subject for delivery eligibility changes.
pub const MAILBOX_WAKE_SUBJECT: &str = "heph.mailbox.v1.wake";
/// Dispatch command subject for one already-eligible mailbox event.
pub const MAILBOX_DISPATCH_SUBJECT: &str = "heph.mailbox.v1.dispatch";
/// Retry command subject for an event whose `PostgreSQL` backoff has elapsed.
pub const MAILBOX_RETRY_SUBJECT: &str = "heph.mailbox.v1.retry";
/// Cancellation command subject for an accepted mailbox event.
pub const MAILBOX_CANCEL_SUBJECT: &str = "heph.mailbox.v1.cancel";
/// Recovery command subject for a stale mailbox claim or incomplete attempt.
pub const MAILBOX_RECOVERY_SUBJECT: &str = "heph.mailbox.v1.recover";

/// All versioned mailbox command subjects owned by this transport.
pub const MAILBOX_COMMAND_SUBJECTS: [&str; 5] = [
    MAILBOX_WAKE_SUBJECT,
    MAILBOX_DISPATCH_SUBJECT,
    MAILBOX_RETRY_SUBJECT,
    MAILBOX_CANCEL_SUBJECT,
    MAILBOX_RECOVERY_SUBJECT,
];

const MAILBOX_COMMAND_STREAM: &str = "HEPH_MAILBOX_COMMANDS";
const MAILBOX_COMMAND_CONSUMER: &str = "mailbox-dispatcher-v1";

/// Creates or resolves the durable mailbox command stream and consumer.
///
/// The stream contains only identifier-only commands. `PostgreSQL` remains
/// authoritative for whether a command is current and eligible when the
/// consumer handles it.
///
/// # Errors
///
/// Returns an error when the `JetStream` account rejects topology creation.
pub async fn ensure_mailbox_jetstream_topology(
    context: &jetstream::Context,
) -> Result<jetstream::consumer::PullConsumer, MailboxTopologyError> {
    use jetstream::stream::{Config, RetentionPolicy, StorageType};

    let stream = context
        .get_or_create_stream(Config {
            name: MAILBOX_COMMAND_STREAM.to_owned(),
            subjects: vec![String::from("heph.mailbox.v1.>")],
            retention: RetentionPolicy::WorkQueue,
            storage: StorageType::File,
            ..Default::default()
        })
        .await
        .map_err(|error| MailboxTopologyError(error.to_string()))?;
    stream
        .get_or_create_consumer(
            MAILBOX_COMMAND_CONSUMER,
            jetstream::consumer::pull::Config {
                durable_name: Some(MAILBOX_COMMAND_CONSUMER.to_owned()),
                filter_subject: String::from("heph.mailbox.v1.>"),
                ack_wait: std::time::Duration::from_secs(30),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| MailboxTopologyError(error.to_string()))
}

/// Mailbox command topology configuration failure.
#[derive(Debug, thiserror::Error)]
#[error("mailbox JetStream topology configuration failed: {0}")]
pub struct MailboxTopologyError(String);

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
}

/// Non-disclosing failure returned by the mailbox authoritative store.
#[derive(Debug, thiserror::Error)]
#[error("mailbox dispatch store failed: {0}")]
pub struct MailboxDispatchStoreError(pub String);

/// Publishes committed mailbox commands with `JetStream` deduplication.
#[derive(Clone)]
pub struct MailboxOutboxPublisher {
    context: jetstream::Context,
    store: Arc<dyn MailboxDispatchStore>,
}

impl MailboxOutboxPublisher {
    /// Creates a publisher using the authoritative mailbox store.
    #[must_use]
    pub fn new(context: jetstream::Context, store: Arc<dyn MailboxDispatchStore>) -> Self {
        Self { context, store }
    }

    /// Publishes up to `limit` committed commands.
    ///
    /// # Errors
    ///
    /// Returns after retaining the first broker or persistence failure.
    pub async fn publish_pending(&self, limit: i64) -> Result<usize, MailboxOutboxPublishError> {
        let rows = self
            .store
            .claim_outbox(&MAILBOX_COMMAND_SUBJECTS, limit)
            .await?;
        let count = rows.len();
        for row in rows {
            let payload = serde_json::to_vec(&row.command)?;
            let mut headers = HeaderMap::new();
            headers.insert("Nats-Msg-Id", row.id.to_string());
            let result = match self
                .context
                .publish_with_headers(row.subject, headers, payload.into())
                .await
            {
                Ok(acknowledgement) => acknowledgement.await.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            match result {
                Ok(_) => {
                    self.store
                        .mark_outbox_published(row.id, row.claim_token)
                        .await?;
                }
                Err(error) => {
                    self.store
                        .mark_outbox_failed(row.id, row.claim_token, &error)
                        .await?;
                    return Err(MailboxOutboxPublishError::JetStream(error));
                }
            }
        }
        Ok(count)
    }
}

/// Mailbox command outbox or `JetStream` publication failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MailboxOutboxPublishError {
    /// The authoritative store failed.
    #[error(transparent)]
    Store(#[from] MailboxDispatchStoreError),
    /// The identifier-only command could not be serialized.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// `JetStream` rejected the publication.
    #[error("JetStream publication failed: {0}")]
    JetStream(String),
}

/// Applies durable mailbox command effects and starts claimed VM runs.
#[derive(Clone)]
pub struct MailboxCommandHandler {
    store: Arc<dyn MailboxDispatchStore>,
    orchestrator: Arc<RunOrchestrator>,
}

/// `JetStream` adapter that acknowledges only after durable mailbox effects.
#[derive(Clone)]
pub struct NatsMailboxCommandHandler {
    handler: MailboxCommandHandler,
}

impl NatsMailboxCommandHandler {
    /// Creates a durable `JetStream` command adapter.
    #[must_use]
    pub const fn new(handler: MailboxCommandHandler) -> Self {
        Self { handler }
    }

    /// Decodes, applies, and confirms one durable delivery.
    ///
    /// # Errors
    ///
    /// Returns without acknowledgement if parsing or any authoritative state
    /// transition fails, allowing `JetStream` to redeliver the same stable
    /// command. The `PostgreSQL` compare-and-swap makes that redelivery safe.
    pub async fn handle(&self, message: &jetstream::Message) -> Result<(), MailboxCommandError> {
        let command: MailboxDispatchCommand = serde_json::from_slice(&message.payload)?;
        self.handler
            .handle(message.message.subject.as_str(), &command)
            .await?;
        message
            .double_ack()
            .await
            .map_err(|error| MailboxCommandError::Acknowledgement(error.to_string()))
    }
}

impl MailboxCommandHandler {
    /// Creates a command handler.
    #[must_use]
    pub fn new(store: Arc<dyn MailboxDispatchStore>, orchestrator: Arc<RunOrchestrator>) -> Self {
        Self {
            store,
            orchestrator,
        }
    }

    /// Handles one command after decoding it from a supported subject.
    ///
    /// The caller must acknowledge a `JetStream` message only after this method
    /// succeeds. Every durable operation is idempotent, so redelivery cannot
    /// create another logical mailbox attempt or VM run.
    ///
    /// # Errors
    ///
    /// Returns an error when the command is unsupported or its durable state
    /// transition or VM orchestration fails.
    pub async fn handle(
        &self,
        subject: &str,
        command: &MailboxDispatchCommand,
    ) -> Result<(), MailboxCommandError> {
        if !MAILBOX_COMMAND_SUBJECTS.contains(&subject) {
            return Err(MailboxCommandError::UnknownSubject(subject.to_owned()));
        }
        self.store.apply_command(subject, command).await?;
        if subject == MAILBOX_DISPATCH_SUBJECT {
            if let Some(run) = self.store.claim_dispatch(command).await? {
                self.orchestrator.start_run(&run).await?;
            }
        }
        Ok(())
    }
}

/// Observes cleaned VM runs and settles their delivery from durable run facts.
#[derive(Clone)]
pub struct MailboxRunCompletion {
    store: Arc<dyn MailboxDispatchStore>,
}

impl MailboxRunCompletion {
    /// Creates a mailbox completion observer.
    #[must_use]
    pub fn new(store: Arc<dyn MailboxDispatchStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl RunCompletionObserver for MailboxRunCompletion {
    async fn after_cleanup(&self, run: &Run) -> Result<(), RunCompletionError> {
        self.store
            .settle_run(run)
            .await
            .map_err(|error| RunCompletionError::redacted(error.to_string()))
    }

    async fn recover(&self) -> Result<usize, RunCompletionError> {
        self.store
            .recover()
            .await
            .map_err(|error| RunCompletionError::redacted(error.to_string()))
    }
}

/// Records mailbox-attempt resource evidence at the VM pre-provision boundary.
#[derive(Clone)]
pub struct MailboxRunResources {
    store: Arc<dyn MailboxDispatchStore>,
}

impl MailboxRunResources {
    /// Creates the pre-provisioning mailbox resource observer.
    #[must_use]
    pub fn new(store: Arc<dyn MailboxDispatchStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl RunResourceObserver for MailboxRunResources {
    async fn record(&self, run: &Run) -> Result<(), RunResourceObservationError> {
        self.store
            .record_run_resources(run)
            .await
            .map_err(|error| RunResourceObservationError::redacted(error.to_string()))
    }
}

/// Durable mailbox command application failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MailboxCommandError {
    /// The identifier-only command payload was malformed.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// The command's store transition failed.
    #[error(transparent)]
    Store(#[from] MailboxDispatchStoreError),
    /// VM orchestration failed and the command must remain eligible for recovery.
    #[error(transparent)]
    Orchestration(#[from] OrchestratorError),
    /// `JetStream` did not confirm the command acknowledgement.
    #[error("JetStream acknowledgement failed: {0}")]
    Acknowledgement(String),
    /// The command subject is not owned by this handler.
    #[error("unsupported mailbox command subject {0}")]
    UnknownSubject(String),
}

#[cfg(test)]
mod tests {
    use super::MailboxDispatchCommand;
    use mailbox_domain::{MailboxEventId, MailboxId, MailboxOperationIdentity};

    #[test]
    fn command_serializes_only_stable_identifiers() {
        let mailbox_id = MailboxId::new();
        let event_id = MailboxEventId::new();
        let command = MailboxDispatchCommand {
            operation_id: MailboxOperationIdentity::dispatch(mailbox_id, event_id, 1).id(),
            event_id,
        };

        let encoded = serde_json::to_value(command).expect("serialize command");
        assert_eq!(encoded.as_object().expect("object").len(), 2);
        assert!(encoded.get("operation_id").is_some());
        assert!(encoded.get("mailbox_event_id").is_some());
    }
}
