use mailbox_dispatch::MailboxDispatchStore;
use mailbox_domain::MailboxId;
use mailbox_postgres::PostgresMailboxRepository;
use serial_test::serial;
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use std::env;
use time::OffsetDateTime;

use super::support::{event, seed_instance};

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn mailbox_payload_retention_purges_only_expired_terminal_body_bytes() {
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
        .expect("apply mailbox migrations");
    let fixture = seed_instance(&pool).await;
    let store = PostgresMailboxRepository::new(pool.clone());
    let mailbox_id = MailboxId::new();
    store
        .ensure_mailbox(fixture.project, mailbox_id, fixture.instance)
        .await
        .expect("create instance-owned mailbox");
    let body = b"retained-until-terminal";
    let accepted = store
        .accept(
            fixture.project,
            &event(mailbox_id, fixture.instance, body),
            body,
            u32::try_from(body.len()).expect("body length"),
        )
        .await
        .expect("accept event");
    // This proof owns retention rather than JetStream publication; settling
    // the wake isolates the shared durable consumer for the transport test.
    sqlx::query("UPDATE outbox SET published_at = now() WHERE id = $1")
        .bind(accepted.event_id.as_uuid())
        .execute(&pool)
        .await
        .expect("isolate retention fixture from transport proof");

    let mut retention = pool.begin().await.expect("begin retention recalculation");
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *retention)
        .await
        .expect("use worker role for retention recalculation");
    sqlx::query("SET LOCAL hephaestus.mailbox_payload_retention_recalculation = 'on'")
        .execute(&mut *retention)
        .await
        .expect("enable narrow retention recalculation");
    sqlx::query("UPDATE mailbox_payloads SET retained_until = now() - interval '1 second' WHERE id = (SELECT body_id FROM mailbox_events WHERE id = $1)")
        .bind(accepted.event_id.as_uuid())
        .execute(&mut *retention)
        .await
        .expect("expire body for proof");
    retention
        .commit()
        .await
        .expect("commit retention recalculation");
    assert_eq!(
        store
            .cleanup_expired_payloads(10)
            .await
            .expect("cleanup pending payload"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, Vec<u8>>("SELECT encoded_body FROM mailbox_payloads WHERE id = (SELECT body_id FROM mailbox_events WHERE id = $1)")
            .bind(accepted.event_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("pending body remains"),
        body
    );

    sqlx::query("UPDATE mailbox_deliveries SET disposition = 'cancelled', terminal_at = now() WHERE event_id = $1")
        .bind(accepted.event_id.as_uuid())
        .execute(&pool)
        .await
        .expect("terminal delivery");
    assert_eq!(
        store
            .cleanup_expired_payloads(10)
            .await
            .expect("cleanup terminal payload"),
        1
    );
    let (body, purged_at, digest): (Option<Vec<u8>>, Option<OffsetDateTime>, Vec<u8>) =
        sqlx::query_as(
            "SELECT encoded_body, body_purged_at, integrity_hash
             FROM mailbox_payloads WHERE id = (SELECT body_id FROM mailbox_events WHERE id = $1)",
        )
        .bind(accepted.event_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("load redacted payload provenance");
    assert!(body.is_none());
    assert!(purged_at.is_some());
    assert_eq!(
        digest,
        Sha256::digest(b"retained-until-terminal").as_slice()
    );
}
