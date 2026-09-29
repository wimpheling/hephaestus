use super::{MAILBOX_COMMAND_CONSUMER, MAILBOX_COMMAND_STREAM};
use async_nats::jetstream;

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
