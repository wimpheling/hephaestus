use futures_util::StreamExt;
use mailbox_dispatch::{
    MAILBOX_DISPATCH_SUBJECT, MAILBOX_WAKE_SUBJECT, MailboxDispatchStore, MailboxOutboxPublisher,
    ensure_mailbox_jetstream_topology,
};
use std::{sync::Arc, time::Duration};

use super::acceptance_setup::AcceptanceState;

// This helper keeps the ordered publish, NAK, reconnect, and claim sequence
// together so the test proves the durable handoff across each transport step.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn run(state: &AcceptanceState, nats_url: &str) -> uuid::Uuid {
    let store = Arc::clone(&state.store);
    let first = &state.first;
    let body = state.body;
    let nats = async_nats::connect(nats_url)
        .await
        .expect("connect real NATS");
    let jetstream = async_nats::jetstream::new(nats);
    let consumer = ensure_mailbox_jetstream_topology(&jetstream)
        .await
        .expect("create durable mailbox consumer");
    let publisher = MailboxOutboxPublisher::new(jetstream.clone(), store.clone());
    // Workspace integration tests share the disposable durable outbox. Drain
    // any earlier mailbox commands, then prove this fixture's exact command.
    assert!(
        publisher
            .publish_pending(1_000)
            .await
            .expect("publish outbox")
            >= 1
    );
    assert_eq!(
        publisher
            .publish_pending(1_000)
            .await
            .expect("replay outbox"),
        0
    );

    let mut messages = consumer.messages().await.expect("open durable consumer");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let first_delivery = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let delivery = tokio::time::timeout(remaining, messages.next())
            .await
            .expect("receive initial JetStream delivery")
            .expect("stream item")
            .expect("valid JetStream message");
        let received: mailbox_dispatch::MailboxDispatchCommand =
            serde_json::from_slice(&delivery.payload).expect("identifier-only command");
        if received.event_id == first.event_id {
            break delivery;
        }
        delivery
            .double_ack()
            .await
            .expect("ack earlier fixture command");
    };
    assert_eq!(
        first_delivery.message.subject.as_str(),
        MAILBOX_WAKE_SUBJECT
    );
    let command: mailbox_dispatch::MailboxDispatchCommand =
        serde_json::from_slice(&first_delivery.payload).expect("identifier-only command");
    assert_eq!(command.event_id, first.event_id);
    assert!(
        !first_delivery
            .payload
            .windows(body.len())
            .any(|window| window == body)
    );
    // NAK models a worker crash after receiving but before acknowledging. The
    // exact same stable command must redeliver, while PostgreSQL remains the
    // authority for its one logical eligibility transition.
    first_delivery
        .ack_with(async_nats::jetstream::AckKind::Nak(None))
        .await
        .expect("request redelivery");
    let redelivery = tokio::time::timeout(Duration::from_secs(5), messages.next())
        .await
        .expect("receive redelivery")
        .expect("stream item")
        .expect("valid redelivery");
    let redelivered: mailbox_dispatch::MailboxDispatchCommand =
        serde_json::from_slice(&redelivery.payload).expect("redelivered command");
    assert_eq!(redelivered, command);
    store
        .apply_command(MAILBOX_WAKE_SUBJECT, &redelivered)
        .await
        .expect("atomically make delivery eligible");
    store
        .apply_command(MAILBOX_WAKE_SUBJECT, &redelivered)
        .await
        .expect("duplicate redelivery is idempotent");
    redelivery
        .double_ack()
        .await
        .expect("ack after PostgreSQL commit");
    // Drop the client-side consumer after the wake-up transition commits.
    // Reconstructing the durable consumer models a dispatcher process restart:
    // the next command is consumed by the reopened durable worker, while
    // PostgreSQL still decides whether it can create a logical attempt.
    drop(messages);
    let consumer = ensure_mailbox_jetstream_topology(&jetstream)
        .await
        .expect("reopen durable mailbox consumer after restart");
    let mut messages = consumer
        .messages()
        .await
        .expect("reopen durable consumer message stream");
    assert_eq!(
        publisher
            .publish_pending(10)
            .await
            .expect("publish dispatch"),
        1
    );
    let dispatch = tokio::time::timeout(Duration::from_secs(5), messages.next())
        .await
        .expect("receive dispatch command")
        .expect("stream item")
        .expect("valid dispatch command");
    assert_eq!(dispatch.message.subject.as_str(), MAILBOX_DISPATCH_SUBJECT);
    let dispatch_command: mailbox_dispatch::MailboxDispatchCommand =
        serde_json::from_slice(&dispatch.payload).expect("dispatch command");
    // A dispatch command is a verified, identifier-only wakeup. Its actual
    // compare-and-swap occurs in `claim_dispatch` immediately afterwards.
    store
        .apply_command(MAILBOX_DISPATCH_SUBJECT, &dispatch_command)
        .await
        .expect("verify committed dispatch command before claiming it");
    let run = store
        .claim_dispatch(&dispatch_command)
        .await
        .expect("durably claim exactly one run")
        .expect("first dispatch creates run");
    assert!(
        store
            .claim_dispatch(&dispatch_command)
            .await
            .expect("duplicate dispatch claim")
            .is_none()
    );
    dispatch
        .double_ack()
        .await
        .expect("ack dispatch after durable claim");
    run.run_id.as_uuid()
}
