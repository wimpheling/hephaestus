use super::support::{request, seed_fixture};
/// Outbox redelivery scenario.
use futures_util::StreamExt;
use gateway_postgres::{GatewayMailboxPublicationResult, PostgresGatewayMailboxPublisher};
use mailbox_dispatch::{
    MAILBOX_WAKE_SUBJECT, MailboxDispatchStore, MailboxOutboxPublisher,
    ensure_mailbox_jetstream_topology,
};
use mailbox_postgres::PostgresMailboxRepository;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc, time::Duration};

#[tokio::test(flavor = "multi_thread")]
#[serial]
#[allow(clippy::too_many_lines)]
async fn gateway_publication_outbox_redelivers_after_dispatcher_crash() {
    let (Ok(database_url), Ok(nats_url)) = (
        env::var("HEPHAESTUS_POSTGRES_TEST_URL"),
        env::var("HEPHAESTUS_NATS_TEST_URL"),
    ) else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway mailbox migrations");
    let fixture = seed_fixture(&pool).await;
    let event_id = match PostgresGatewayMailboxPublisher::new(pool.clone())
        .publish(request(&fixture, "deliver", "crash-recovery"))
        .await
        .expect("accept gateway publication before dispatcher crash")
    {
        GatewayMailboxPublicationResult::Accepted { event_id } => event_id,
        outcome => panic!("expected accepted publication, got {outcome:?}"),
    };

    let nats = async_nats::connect(nats_url)
        .await
        .expect("connect real NATS");
    let jetstream = async_nats::jetstream::new(nats);
    let consumer = ensure_mailbox_jetstream_topology(&jetstream)
        .await
        .expect("create durable mailbox consumer");
    let store = Arc::new(PostgresMailboxRepository::new(pool.clone()));
    let outbox = MailboxOutboxPublisher::new(jetstream.clone(), store.clone());
    assert!(
        outbox
            .publish_pending(1_000)
            .await
            .expect("publish committed gateway mailbox wake")
            >= 1
    );

    let mut messages = consumer.messages().await.expect("open mailbox consumer");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let first = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let message = tokio::time::timeout(remaining, messages.next())
            .await
            .expect("receive initial mailbox wake")
            .expect("mailbox stream item")
            .expect("valid mailbox wake");
        let command: mailbox_dispatch::MailboxDispatchCommand =
            serde_json::from_slice(&message.payload).expect("identifier-only mailbox command");
        if command.event_id == event_id {
            assert_eq!(message.message.subject.as_str(), MAILBOX_WAKE_SUBJECT);
            break (message, command);
        }
        message
            .double_ack()
            .await
            .expect("ack earlier fixture command");
    };
    // A NAK is the exact broker-visible shape of a dispatcher process dying
    // after receive and before it durably applies the wake transition.
    first
        .0
        .ack_with(async_nats::jetstream::AckKind::Nak(None))
        .await
        .expect("model dispatcher crash before acknowledgement");
    let redelivery = tokio::time::timeout(Duration::from_secs(5), messages.next())
        .await
        .expect("receive redelivered gateway mailbox wake")
        .expect("mailbox stream item")
        .expect("valid redelivered wake");
    let redelivered: mailbox_dispatch::MailboxDispatchCommand =
        serde_json::from_slice(&redelivery.payload).expect("identifier-only redelivery");
    assert_eq!(redelivered, first.1);
    assert!(
        !redelivery
            .payload
            .windows(b"gateway-postgres-proof".len())
            .any(|window| window == b"gateway-postgres-proof"),
        "JetStream command must not expose the gateway body"
    );
    store
        .apply_command(MAILBOX_WAKE_SUBJECT, &redelivered)
        .await
        .expect("apply redelivered wake durably");
    store
        .apply_command(MAILBOX_WAKE_SUBJECT, &redelivered)
        .await
        .expect("duplicate redelivery stays idempotent");
    redelivery
        .double_ack()
        .await
        .expect("ack only after durable recovery");

    let (events, publications, deliveries): (i64, i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM mailbox_events WHERE id = $1),
             (SELECT count(*) FROM gateway_mailbox_publications WHERE event_id = $1),
             (SELECT count(*) FROM mailbox_deliveries WHERE event_id = $1)",
    )
    .bind(event_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("gateway crash recovery remains one logical event");
    assert_eq!((events, publications, deliveries), (1, 1, 1));
}
