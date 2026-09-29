use super::{
    MAILBOX_COMMAND_SUBJECTS, MAILBOX_DISPATCH_SUBJECT,
    contract::{MailboxDispatchCommand, MailboxDispatchStore, MailboxDispatchStoreError},
};
use async_nats::jetstream;
use run_orchestrator::{OrchestratorError, RunOrchestrator};
use std::sync::Arc;

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
        tracing::debug!(
            mailbox_event_id = %command.event_id,
            mailbox_operation_id = %command.operation_id,
            subject,
            "applying durable mailbox command"
        );
        self.store.apply_command(subject, command).await?;
        if subject == MAILBOX_DISPATCH_SUBJECT {
            if let Some(run) = self.store.claim_dispatch(command).await? {
                tracing::info!(
                    mailbox_event_id = %command.event_id,
                    mailbox_operation_id = %command.operation_id,
                    run_id = %run.run_id,
                    instance_id = %run.instance_id,
                    instance_revision_id = %run.instance_revision_id,
                    "starting mailbox-dispatched run"
                );
                self.orchestrator.start_run(&run).await?;
            }
        }
        Ok(())
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
