//! mailbox publication scenario.

use super::support::{request, revoke_grant, seed_fixture};
use gateway_postgres::{GatewayMailboxPublicationResult, PostgresGatewayMailboxPublisher};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::env;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
#[allow(clippy::too_many_lines)]
async fn gateway_mailbox_publication_rechecks_live_grants_slots_and_producer_scope() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
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
    let publisher = PostgresGatewayMailboxPublisher::new(pool.clone());

    let accepted = publisher
        .publish(request(&fixture, "deliver", "first"))
        .await
        .expect("publish with a live exact binding and grant");
    let event_id = match accepted {
        GatewayMailboxPublicationResult::Accepted { event_id } => event_id,
        other => panic!("expected accepted publication, got {other:?}"),
    };
    let duplicate = publisher
        .publish(request(&fixture, "deliver", "first"))
        .await
        .expect("repeat publication is idempotent");
    assert_eq!(
        duplicate,
        GatewayMailboxPublicationResult::Duplicate { event_id },
        "the same invocation/slot/key must retain the accepted event"
    );
    let (events, wakes, publications): (i64, i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM mailbox_events WHERE id = $1),
             (SELECT count(*) FROM outbox WHERE id = $1 AND subject = 'heph.mailbox.v1.wake'),
             (SELECT count(*) FROM gateway_mailbox_publications
                 WHERE invocation_id = $2 AND slot_key = 'deliver' AND deduplication_key = 'first')",
    )
    .bind(event_id.as_uuid())
    .bind(fixture.invocation)
    .fetch_one(&pool)
    .await
    .expect("load durable acceptance evidence");
    assert_eq!((events, wakes, publications), (1, 1, 1));

    // A declared but unbound slot cannot borrow this binding's mailbox grant.
    assert_eq!(
        publisher
            .publish(request(&fixture, "other", "wrong-slot"))
            .await
            .expect("wrong slot is a redacted denial"),
        GatewayMailboxPublicationResult::Denied
    );

    // Runtime sessions are not ambient authority: pausing the gateway after
    // issuance prevents a fresh invocation from publishing through its old
    // immutable revision.
    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(fixture.gateway)
        .execute(&pool)
        .await
        .expect("pause gateway");
    assert_eq!(
        publisher
            .publish(request(&fixture, "deliver", "gateway-paused"))
            .await
            .expect("paused gateway is a redacted denial"),
        GatewayMailboxPublicationResult::Denied
    );
    sqlx::query("UPDATE gateways SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.gateway)
        .execute(&pool)
        .await
        .expect("resume gateway for grant revocation proof");

    revoke_grant(&pool, fixture.grant, fixture.owner).await;
    assert_eq!(
        publisher
            .publish(request(&fixture, "deliver", "after-revoke"))
            .await
            .expect("revoked grant is a redacted denial"),
        GatewayMailboxPublicationResult::Denied
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1 AND producer_id = 'gateway-proof'",
        )
        .bind(fixture.mailbox)
        .fetch_one(&pool)
        .await
        .expect("count only accepted mailbox events"),
        1
    );

    // Producer identity is globally unique per mailbox, so a second gateway
    // binding cannot impersonate or collide with this producer scope.
    let collision = sqlx::query(
        "INSERT INTO gateway_mailbox_bindings
             (id, gateway_revision_id, gateway_id, project_id, slot_key, mailbox_id, producer_id, created_by)
         VALUES ($1, $2, $3, $4, 'other', $5, 'gateway-proof', $6)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.revision)
    .bind(fixture.gateway)
    .bind(fixture.project)
    .bind(fixture.mailbox)
    .bind(fixture.owner)
    .execute(&pool)
    .await;
    assert!(collision.is_err(), "mailbox producer scopes must be unique");
}
