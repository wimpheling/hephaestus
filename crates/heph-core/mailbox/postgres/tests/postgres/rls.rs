use mailbox_domain::MailboxId;
use mailbox_postgres::PostgresMailboxRepository;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::env;

use super::support::{event, seed_instance, visible_event_count};

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn mailbox_rls_isolates_tenants_and_preserves_removed_owner_history() {
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
    let body = b"tombstone-safe-mailbox-history";
    let accepted = store
        .accept(
            fixture.project,
            &event(mailbox_id, fixture.instance, body),
            body,
            u32::try_from(body.len()).expect("body length"),
        )
        .await
        .expect("accept event before removal");
    // This test deliberately stops at persistence and inspection.  Mark its
    // wake command settled so the separate transport test owns the shared
    // disposable JetStream consumer regardless of Tokio test ordering.
    sqlx::query("UPDATE outbox SET published_at = now() WHERE id = $1")
        .bind(accepted.event_id.as_uuid())
        .execute(&pool)
        .await
        .expect("isolate RLS fixture from transport proof");

    assert_eq!(
        visible_event_count(&pool, &fixture.owner, mailbox_id).await,
        1
    );
    assert_eq!(
        visible_event_count(&pool, &fixture.outsider, mailbox_id).await,
        0
    );

    // A mailbox tombstone stops new acceptance without deleting immutable event
    // history.  This mirrors a removed instance whose previous delivery/audit
    // evidence must remain available to authorized operators.
    sqlx::query("UPDATE mailboxes SET state = 'removed', removed_at = now() WHERE id = $1")
        .bind(mailbox_id.as_uuid())
        .execute(&pool)
        .await
        .expect("tombstone mailbox");
    assert!(
        store
            .accept(
                fixture.project,
                &event(mailbox_id, fixture.instance, b"must-not-be-accepted"),
                b"must-not-be-accepted",
                20,
            )
            .await
            .is_err()
    );
    assert_eq!(
        visible_event_count(&pool, &fixture.owner, mailbox_id).await,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mailbox_events WHERE id = $1")
            .bind(accepted.event_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("read immutable retained event"),
        1
    );
}
