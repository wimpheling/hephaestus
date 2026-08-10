//! The single typed product-event adapter between committed `PostgreSQL` events
//! and the external `JetStream` subject.

// This private module's crate-visible items are the intentional adapter API.
#![allow(clippy::redundant_pub_crate)]

use crate::{
    application::event::{
        ApplicationEvent, EventScope, EventWakeupSource, EventWakeupStream, ScopeKind,
    },
    event_cursor::EventCursorCodec,
    rpc::{RpcError, event::model},
};
use async_nats::{HeaderMap, jetstream};
use buffa::Message as _;
use event_application::{ProductEventOutbox, ProductEventRecord};
use futures_util::StreamExt as _;
use std::sync::Arc;
use std::time::Duration;

pub(crate) const PRODUCT_EVENT_SUBJECT: &str = "hephaestus.product.event.v1";
const PRODUCT_EVENT_STREAM: &str = "HEPHAESTUS_PRODUCT_EVENTS";
const PRODUCT_EVENT_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const PRODUCT_EVENT_MAX_MESSAGES: i64 = 1_000_000;
const PRODUCT_EVENT_MAX_BYTES: i64 = 1024 * 1024 * 1024;
const PRODUCT_EVENT_MAX_MESSAGE_SIZE: i32 = 64 * 1024;
const PRODUCT_EVENT_DUPLICATE_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);

pub(crate) struct NatsEventWakeups {
    client: async_nats::Client,
}

impl NatsEventWakeups {
    pub(crate) const fn new(client: async_nats::Client) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl EventWakeupSource for NatsEventWakeups {
    async fn subscribe(&self) -> Result<EventWakeupStream, String> {
        let subscriber = self
            .client
            .subscribe(PRODUCT_EVENT_SUBJECT)
            .await
            .map_err(|error| error.to_string())?;
        Ok(Box::pin(subscriber.map(|_message| ())))
    }
}

pub(crate) async fn ensure_topology(context: &jetstream::Context) -> Result<(), EventPublishError> {
    context
        .create_or_update_stream(product_event_stream_config())
        .await
        .map_err(|error| EventPublishError::JetStream(error.to_string()))?;
    Ok(())
}

fn product_event_stream_config() -> jetstream::stream::Config {
    use jetstream::stream::{Config, RetentionPolicy, StorageType};

    Config {
        name: PRODUCT_EVENT_STREAM.to_owned(),
        description: Some(String::from(
            "Bounded wake and at-least-once transport for durable product events",
        )),
        subjects: vec![PRODUCT_EVENT_SUBJECT.to_owned()],
        retention: RetentionPolicy::Limits,
        storage: StorageType::File,
        max_age: PRODUCT_EVENT_MAX_AGE,
        max_messages: PRODUCT_EVENT_MAX_MESSAGES,
        max_bytes: PRODUCT_EVENT_MAX_BYTES,
        max_message_size: PRODUCT_EVENT_MAX_MESSAGE_SIZE,
        duplicate_window: PRODUCT_EVENT_DUPLICATE_WINDOW,
        ..Default::default()
    }
}

#[derive(Clone)]
pub(crate) struct EventPublisher {
    context: jetstream::Context,
    outbox: Arc<dyn ProductEventOutbox>,
    cursor_codec: EventCursorCodec,
}

impl EventPublisher {
    pub(crate) const fn new(
        context: jetstream::Context,
        outbox: Arc<dyn ProductEventOutbox>,
        cursor_key: [u8; 32],
    ) -> Self {
        Self {
            context,
            outbox,
            cursor_codec: EventCursorCodec::new(cursor_key),
        }
    }

    pub(crate) async fn publish_pending(&self, limit: i64) -> Result<usize, EventPublishError> {
        if limit <= 0 {
            return Ok(0);
        }
        let rows = self.outbox.pending(limit).await.map_err(provider)?;
        let count = rows.len();
        for row in rows {
            let event_id = row.id;
            let product = match into_product_event(row, &self.cursor_codec) {
                Ok(product) => product,
                Err(EventPublishError::Projection) => {
                    self.outbox
                        .dead_letter(event_id, "projection failure")
                        .await
                        .map_err(provider)?;
                    tracing::error!(%event_id, "dead-lettered invalid persisted product event");
                    continue;
                }
                Err(error) => return Err(error),
            };
            let mut headers = HeaderMap::new();
            headers.insert("Nats-Msg-Id", event_id.to_string());
            let result = match self
                .context
                .publish_with_headers(PRODUCT_EVENT_SUBJECT, headers, product.encode_to_bytes())
                .await
            {
                Ok(acknowledgement) => acknowledgement.await.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            match result {
                Ok(_) => {
                    self.outbox
                        .mark_published(event_id)
                        .await
                        .map_err(provider)?;
                }
                Err(error) => {
                    self.outbox
                        .mark_failed(event_id, &error)
                        .await
                        .map_err(provider)?;
                    return Err(EventPublishError::JetStream(error));
                }
            }
        }
        Ok(count)
    }
}

fn into_product_event(
    record: ProductEventRecord,
    codec: &EventCursorCodec,
) -> Result<rpc_proto::messages::hephaestus::event::v1::ProductEvent, EventPublishError> {
    let scope = EventScope {
        kind: match record.scope_kind.as_str() {
            "identity" => ScopeKind::Identity,
            "organization" => ScopeKind::Organization,
            "project" => ScopeKind::Project,
            "repository" => ScopeKind::Repository,
            "run" => ScopeKind::Run,
            "agent_instance" => ScopeKind::AgentInstance,
            _ => return Err(EventPublishError::Projection),
        },
        id: record.scope_id,
    };
    model::event(
        codec,
        scope,
        &ApplicationEvent {
            id: record.id,
            cursor: record.cursor,
            aggregate_type: record.aggregate_type,
            aggregate_id: record.aggregate_id,
            aggregate_version: record.aggregate_version,
            event_type: record.event_type,
            schema_version: record.schema_version,
            change_kind: record.change_kind,
            safe_state: record.safe_state,
            related_id_one: record.related_id_one,
            related_id_two: record.related_id_two,
            actor_id: record.actor_id,
            request_id: record.request_id,
            occurred_at: record.occurred_at,
        },
    )
    .map_err(|_: RpcError| EventPublishError::Projection)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum EventPublishError {
    #[error("product event provider failed: {0}")]
    Provider(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("product event projection rejected persisted data")]
    Projection,
    #[error("product event JetStream operation failed: {0}")]
    JetStream(String),
}

fn provider(error: impl std::error::Error + Send + Sync + 'static) -> EventPublishError {
    EventPublishError::Provider(Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::{
        PRODUCT_EVENT_DUPLICATE_WINDOW, PRODUCT_EVENT_MAX_AGE, PRODUCT_EVENT_MAX_BYTES,
        PRODUCT_EVENT_MAX_MESSAGE_SIZE, PRODUCT_EVENT_MAX_MESSAGES, PRODUCT_EVENT_STREAM,
        PRODUCT_EVENT_SUBJECT, product_event_stream_config,
    };
    use async_nats::jetstream::stream::{RetentionPolicy, StorageType};

    #[test]
    fn product_event_stream_has_finite_transport_retention() {
        let config = product_event_stream_config();

        assert_eq!(config.name, PRODUCT_EVENT_STREAM);
        assert_eq!(config.subjects, [PRODUCT_EVENT_SUBJECT]);
        assert_eq!(config.retention, RetentionPolicy::Limits);
        assert_eq!(config.storage, StorageType::File);
        assert_eq!(config.max_age, PRODUCT_EVENT_MAX_AGE);
        assert_eq!(config.max_messages, PRODUCT_EVENT_MAX_MESSAGES);
        assert_eq!(config.max_bytes, PRODUCT_EVENT_MAX_BYTES);
        assert_eq!(config.max_message_size, PRODUCT_EVENT_MAX_MESSAGE_SIZE);
        assert_eq!(config.duplicate_window, PRODUCT_EVENT_DUPLICATE_WINDOW);
        assert!(config.duplicate_window <= config.max_age);
        assert!(config.max_message_size > 0);
        assert!(config.max_bytes >= i64::from(config.max_message_size));
    }
}
