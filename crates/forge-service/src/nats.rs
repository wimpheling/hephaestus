use async_nats::{HeaderMap, jetstream};

use crate::{ForgeRepositoryError, PgForgeRepository};

/// Accepted Git receive audit events.
pub const GIT_RECEIVE_ACCEPTED_SUBJECT: &str = "hephaestus.git.receive.accepted";
/// Invalid repository agent configuration events.
pub const AGENT_CONFIG_INVALID_SUBJECT: &str = "hephaestus.git.agent_config.invalid";
/// Durable commands consumed by the run orchestrator.
pub const RUN_START_SUBJECT: &str = "hephaestus.run.start";

const GIT_EVENT_STREAM: &str = "HEPHAESTUS_GIT_EVENTS";

/// Creates the durable Git event stream.
///
/// The run-command stream is owned by `run-orchestrator`, which includes
/// [`RUN_START_SUBJECT`] in its topology.
///
/// # Errors
///
/// Returns an error when `JetStream` rejects topology creation.
pub async fn ensure_forge_jetstream_topology(
    context: &jetstream::Context,
) -> Result<(), ForgeOutboxPublishError> {
    use jetstream::stream::{Config, RetentionPolicy, StorageType};

    context
        .get_or_create_stream(Config {
            name: GIT_EVENT_STREAM.to_owned(),
            subjects: vec![
                GIT_RECEIVE_ACCEPTED_SUBJECT.to_owned(),
                AGENT_CONFIG_INVALID_SUBJECT.to_owned(),
            ],
            retention: RetentionPolicy::Limits,
            storage: StorageType::File,
            ..Default::default()
        })
        .await
        .map_err(|error| ForgeOutboxPublishError::JetStream(error.to_string()))?;
    Ok(())
}

/// Publishes committed forge outbox records to `JetStream`.
#[derive(Clone)]
pub struct ForgeNatsOutboxPublisher {
    context: jetstream::Context,
}

impl ForgeNatsOutboxPublisher {
    /// Creates a publisher.
    #[must_use]
    pub const fn new(context: jetstream::Context) -> Self {
        Self { context }
    }

    /// Publishes and marks up to `limit` pending outbox records.
    ///
    /// # Errors
    ///
    /// Returns after recording the first publication failure.
    pub async fn publish_pending(
        &self,
        repository: &PgForgeRepository,
        limit: i64,
    ) -> Result<usize, ForgeOutboxPublishError> {
        let records = repository.unpublished_outbox(limit).await?;
        let count = records.len();
        for record in records {
            let payload = serde_json::to_vec(&record.payload)?;
            let mut headers = HeaderMap::new();
            headers.insert("Nats-Msg-Id", record.id.to_string());
            let result = match self
                .context
                .publish_with_headers(record.subject, headers, payload.into())
                .await
            {
                Ok(acknowledgement) => acknowledgement.await.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            match result {
                Ok(_) => repository.mark_outbox_published(record.id).await?,
                Err(error) => {
                    repository.mark_outbox_failed(record.id, &error).await?;
                    return Err(ForgeOutboxPublishError::JetStream(error));
                }
            }
        }
        Ok(count)
    }
}

/// Forge outbox topology or publication failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ForgeOutboxPublishError {
    /// Forge repository access failed.
    #[error(transparent)]
    Repository(#[from] ForgeRepositoryError),
    /// JSON serialization failed.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// `JetStream` operation failed.
    #[error("JetStream operation failed: {0}")]
    JetStream(String),
}
