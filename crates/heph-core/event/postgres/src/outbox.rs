use async_nats::{HeaderMap, jetstream};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

const RELEASE_EVENT_STREAM: &str = "HEPHAESTUS_RELEASE_EVENTS";

/// Creates the durable stream for bounded, non-UI release lifecycle records.
///
/// Actionable run and instance-trigger subjects are owned by their existing
/// command streams and are intentionally excluded here.
///
/// # Errors
///
/// Returns an error when `JetStream` rejects topology creation.
pub async fn ensure_release_jetstream_topology(
    context: &jetstream::Context,
) -> Result<(), ReleaseOutboxPublishError> {
    use jetstream::stream::{Config, RetentionPolicy, StorageType};

    context
        .get_or_create_stream(Config {
            name: RELEASE_EVENT_STREAM.to_owned(),
            subjects: vec![
                String::from("hephaestus.build.completed.v1"),
                String::from("hephaestus.build.failed.v1"),
                String::from("hephaestus.release.>"),
                String::from("hephaestus.agent_instance.>"),
                String::from("hephaestus.agent_update.>"),
            ],
            retention: RetentionPolicy::Limits,
            storage: StorageType::File,
            ..Default::default()
        })
        .await
        .map_err(|error| ReleaseOutboxPublishError::JetStream(error.to_string()))?;
    Ok(())
}

/// Publishes release-owned transactional outbox records to `JetStream`.
#[derive(Clone)]
pub struct ReleaseOutboxPublisher {
    context: jetstream::Context,
    pool: PgPool,
}

impl ReleaseOutboxPublisher {
    /// Creates a publisher for release-owned records.
    #[must_use]
    pub const fn new(context: jetstream::Context, pool: PgPool) -> Self {
        Self { context, pool }
    }

    /// Publishes and marks up to `limit` pending records.
    ///
    /// # Errors
    ///
    /// Returns after recording the first database or publication failure.
    pub async fn publish_pending(&self, limit: i64) -> Result<usize, ReleaseOutboxPublishError> {
        let rows = sqlx::query_as::<_, ReleaseOutboxRow>(
            "SELECT id, subject, payload FROM outbox
             WHERE published_at IS NULL AND aggregate_type = 'release'
             ORDER BY occurred_at, id LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let count = rows.len();
        for row in rows {
            let payload = serde_json::to_vec(&row.payload)?;
            let mut headers = HeaderMap::new();
            headers.insert("Nats-Msg-Id", row.id.to_string());
            let publication = self
                .context
                .publish_with_headers(row.subject, headers, payload.into())
                .await;
            let result = match publication {
                Ok(acknowledgement) => acknowledgement.await.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            match result {
                Ok(_) => {
                    sqlx::query(
                        "UPDATE outbox
                         SET published_at = now(), attempts = attempts + 1,
                             last_error = NULL
                         WHERE id = $1",
                    )
                    .bind(row.id)
                    .execute(&self.pool)
                    .await?;
                }
                Err(error) => {
                    sqlx::query(
                        "UPDATE outbox
                         SET attempts = attempts + 1, last_error = $2
                         WHERE id = $1",
                    )
                    .bind(row.id)
                    .bind(&error)
                    .execute(&self.pool)
                    .await?;
                    return Err(ReleaseOutboxPublishError::JetStream(error));
                }
            }
        }
        Ok(count)
    }
}

#[derive(sqlx::FromRow)]
struct ReleaseOutboxRow {
    id: Uuid,
    subject: String,
    payload: Value,
}

/// Release outbox database, serialization, or publication failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReleaseOutboxPublishError {
    /// `PostgreSQL` access failed.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    /// Command serialization failed.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// `JetStream` rejected publication.
    #[error("JetStream publication failed: {0}")]
    JetStream(String),
}
