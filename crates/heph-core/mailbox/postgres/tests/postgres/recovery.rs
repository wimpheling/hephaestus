use mailbox_dispatch::MailboxDispatchStore;
use mailbox_domain::MailboxId;
use mailbox_postgres::PostgresMailboxRepository;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc, time::Duration};

use super::support::{seed_instance, seed_running_run};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn recovery_does_not_classify_a_run_cleaned_during_its_fallback_statement() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!(
            "SKIP mailbox recovery cleanup race integration: HEPHAESTUS_POSTGRES_TEST_URL is unset"
        );
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
    let store = Arc::new(PostgresMailboxRepository::new(pool.clone()));
    let mailbox_id = MailboxId::new();
    store
        .ensure_mailbox(fixture.project, mailbox_id, fixture.instance)
        .await
        .expect("create race-test mailbox");
    let (target_event_id, target_run_id) = seed_running_run(
        &store,
        &pool,
        fixture.project,
        mailbox_id,
        fixture.instance,
        fixture.revision,
        b"recovery-cleanup-read-committed-target",
    )
    .await;
    let (blocked_event_id, blocked_run_id) = seed_running_run(
        &store,
        &pool,
        fixture.project,
        mailbox_id,
        fixture.instance,
        fixture.revision,
        b"recovery-cleanup-read-committed-blocker",
    )
    .await;
    sqlx::query("UPDATE runs SET state = 'cleaned_up', outcome = 'succeeded' WHERE id = $1")
        .bind(blocked_run_id)
        .execute(&pool)
        .await
        .expect("prepare committed cleaned blocker run");

    // Hold the already-cleaned blocker row so recover has taken its initial
    // READ COMMITTED snapshot (where the target run is still running) and is
    // waiting at the completed-run reconciliation query. Cleanup can commit
    // the target independently while that statement is blocked, making the
    // stale-snapshot ordering deterministic without a production hook.
    let mut blocker = pool.begin().await.expect("begin delivery lock");
    sqlx::query("SELECT event_id FROM mailbox_deliveries WHERE event_id = $1 FOR UPDATE")
        .bind(blocked_event_id.as_uuid())
        .fetch_one(&mut *blocker)
        .await
        .expect("lock cleaned blocker delivery");
    let recovery = {
        let store = Arc::clone(&store);
        tokio::spawn(async move { store.recover().await })
    };
    let reached_reconciliation_wait = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                     SELECT 1 FROM pg_stat_activity
                      WHERE datname = current_database() AND pid <> pg_backend_pid()
                        AND state = 'active'
                        AND wait_event_type = 'Lock'
                        AND query LIKE '%FOR UPDATE OF delivery, attempt%'
                 )",
            )
            .fetch_one(&pool)
            .await
            .expect("inspect recovery wait");
            if waiting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok();
    if !reached_reconciliation_wait {
        drop(blocker);
        let _ = tokio::time::timeout(Duration::from_secs(5), recovery).await;
        panic!("recover did not reach the blocked reconciliation query");
    }
    sqlx::query("UPDATE runs SET state = 'cleaned_up', outcome = 'succeeded' WHERE id = $1")
        .bind(target_run_id)
        .execute(&pool)
        .await
        .expect("commit cleanup while recover is blocked");
    blocker.commit().await.expect("release delivery lock");
    tokio::time::timeout(Duration::from_secs(5), recovery)
        .await
        .expect("recover did not finish after barrier release")
        .expect("join recovery")
        .expect("recover race-test transaction");

    let (disposition, attempt_state): (String, String) = sqlx::query_as(
        "SELECT delivery.disposition, attempt.state
           FROM mailbox_deliveries AS delivery
           JOIN mailbox_delivery_attempts AS attempt ON attempt.event_id = delivery.event_id
          WHERE delivery.event_id = $1",
    )
    .bind(target_event_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("read race-test state");
    assert_eq!(disposition, "leased");
    assert_eq!(attempt_state, "running");

    // The next pass sees the now-committed cleaned target in its first
    // statement and performs the normal idempotent successful settlement.
    assert_eq!(
        store.recover().await.expect("settle cleaned race-test run"),
        0
    );
    let (disposition, attempt_state): (String, String) = sqlx::query_as(
        "SELECT delivery.disposition, attempt.state
           FROM mailbox_deliveries AS delivery
           JOIN mailbox_delivery_attempts AS attempt ON attempt.event_id = delivery.event_id
          WHERE delivery.event_id = $1",
    )
    .bind(target_event_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("read settled race-test state");
    assert_eq!(disposition, "delivered");
    assert_eq!(attempt_state, "completed");
}
