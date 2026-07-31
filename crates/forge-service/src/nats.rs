use async_nats::{HeaderMap, jetstream};

use crate::{ForgeOutboxStore, ForgeRepositoryError};

/// Durable isolated-build requests for reusable releases.
pub const BUILD_REQUESTED_SUBJECT: &str = "hephaestus.build.requested.v1";
/// Exact reusable-instance run requests awaiting dispatch.
pub const INSTANCE_RUN_REQUESTED_SUBJECT: &str = "hephaestus.instance.run.requested.v1";
/// Durable commands consumed by the run orchestrator.
pub const RUN_START_SUBJECT: &str = "hephaestus.run.start";

const GIT_EVENT_STREAM: &str = "HEPHAESTUS_GIT_EVENTS";
const BUILD_CONSUMER: &str = "isolated-build-executor-v1";

/// Creates the durable forge command stream.
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
                BUILD_REQUESTED_SUBJECT.to_owned(),
                INSTANCE_RUN_REQUESTED_SUBJECT.to_owned(),
            ],
            retention: RetentionPolicy::Limits,
            storage: StorageType::File,
            ..Default::default()
        })
        .await
        .map_err(|error| ForgeOutboxPublishError::JetStream(error.to_string()))?;
    Ok(())
}

/// Creates or resolves the durable isolated-build request consumer.
///
/// # Errors
///
/// Returns an error when the Git event stream or consumer is unavailable.
pub async fn ensure_build_consumer(
    context: &jetstream::Context,
) -> Result<jetstream::consumer::PullConsumer, ForgeOutboxPublishError> {
    let stream = context
        .get_stream(GIT_EVENT_STREAM)
        .await
        .map_err(|error| ForgeOutboxPublishError::JetStream(error.to_string()))?;
    stream
        .get_or_create_consumer(
            BUILD_CONSUMER,
            jetstream::consumer::pull::Config {
                durable_name: Some(BUILD_CONSUMER.to_owned()),
                filter_subject: BUILD_REQUESTED_SUBJECT.to_owned(),
                ack_wait: std::time::Duration::from_secs(30),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| ForgeOutboxPublishError::JetStream(error.to_string()))
}

/// Publishes committed forge commands to `JetStream`.
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
        repository: &dyn ForgeOutboxStore,
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
