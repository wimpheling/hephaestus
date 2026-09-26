use mailbox_dispatch::MAILBOX_WAKE_SUBJECT;
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, EnvelopeMethod, EnvelopeRoute,
    MailboxEnvelope, MailboxEvent, MailboxEventId, MailboxId,
};
use mailbox_postgres::{AcceptedMailboxEvent, PostgresMailboxRepository};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{collections::BTreeMap, sync::Arc};
use time::OffsetDateTime;

use super::support::{Fixture, event, seed_instance};

pub struct AcceptanceState {
    pub pool: PgPool,
    pub fixture: Fixture,
    pub store: Arc<PostgresMailboxRepository>,
    pub mailbox_id: MailboxId,
    pub body: &'static [u8],
    pub first: AcceptedMailboxEvent,
}

pub async fn setup(database_url: &str) -> AcceptanceState {
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply mailbox migrations");
    let fixture = seed_instance(&pool).await;
    let store = Arc::new(PostgresMailboxRepository::new(pool.clone()));
    let mailbox_id = MailboxId::new();
    store
        .ensure_mailbox(fixture.project, mailbox_id, fixture.instance)
        .await
        .expect("create instance-owned mailbox");
    let instance_watch_events_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE scope_kind = 'agent_instance' AND scope_id = $1
           AND aggregate_type = 'agent_instance'
           AND event_type = 'agent_instance.changed'",
    )
    .bind(fixture.instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("load initial instance watch cursor");
    let body = b"real-postgres-nats-mailbox";
    let first_event = event(mailbox_id, fixture.instance, body);
    let duplicate_event = MailboxEvent {
        id: MailboxEventId::new(),
        envelope: MailboxEnvelope::new(
            EnvelopeMethod::parse("POST").expect("method"),
            EnvelopeRoute::parse("/mailbox/proof").expect("route"),
            BTreeMap::default(),
            ContentMetadata::new(
                BodyReference::new(
                    BodyReferenceId::new(),
                    u32::try_from(body.len()).expect("body length"),
                    Sha256::digest(body).into(),
                )
                .expect("body reference"),
                Some("application/octet-stream".to_owned()),
                Some("identity".to_owned()),
            )
            .expect("metadata"),
            OffsetDateTime::now_utc(),
            None,
        )
        .expect("envelope"),
        ..first_event.clone()
    };
    let decoded_length = u32::try_from(body.len()).expect("body length");
    let (first, duplicate) = tokio::join!(
        store.accept(fixture.project, &first_event, body, decoded_length),
        store.accept(fixture.project, &duplicate_event, body, decoded_length),
    );
    let first = first.expect("first durable acceptance");
    let duplicate = duplicate.expect("concurrent duplicate acceptance");
    assert_eq!(first.event_id, duplicate.event_id);
    assert_ne!(first.duplicate, duplicate.duplicate);
    validate_acceptance(
        &pool,
        &fixture,
        mailbox_id,
        &first,
        instance_watch_events_before,
    )
    .await;

    AcceptanceState {
        pool,
        fixture,
        store,
        mailbox_id,
        body,
        first,
    }
}

async fn validate_acceptance(
    pool: &PgPool,
    fixture: &Fixture,
    mailbox_id: MailboxId,
    first: &AcceptedMailboxEvent,
    instance_watch_events_before: i64,
) {
    let (payloads, events, deliveries, wake_outbox): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM mailbox_payloads WHERE mailbox_id = $1),
             (SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1),
             (SELECT count(*) FROM mailbox_deliveries WHERE event_id = $2),
             (SELECT count(*) FROM outbox WHERE id = $2 AND subject = $3)",
    )
    .bind(mailbox_id.as_uuid())
    .bind(first.event_id.as_uuid())
    .bind(MAILBOX_WAKE_SUBJECT)
    .fetch_one(pool)
    .await
    .expect("load atomic acceptance evidence");
    assert_eq!((payloads, events, deliveries, wake_outbox), (1, 1, 1, 1));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM application_events
             WHERE scope_kind = 'agent_instance' AND scope_id = $1
               AND aggregate_type = 'agent_instance'
               AND event_type = 'agent_instance.changed'",
        )
        .bind(fixture.instance.as_uuid())
        .fetch_one(pool)
        .await
        .expect("mailbox delivery emits a reauthorizable instance wake"),
        instance_watch_events_before + 1
    );
    let (queue_depth, acceptance_to_dispatch_milliseconds): (i64, i64) = sqlx::query_as(
        "SELECT queue_depth, acceptance_to_dispatch_milliseconds
         FROM mailbox_operation_metrics",
    )
    .fetch_one(pool)
    .await
    .expect("read aggregate-only mailbox operational metrics");
    assert!(queue_depth >= 1);
    // This aggregate includes durable attempts created by earlier workspace
    // fixtures, so only its non-negative timing invariant is local here.
    assert!(acceptance_to_dispatch_milliseconds >= 0);
}
