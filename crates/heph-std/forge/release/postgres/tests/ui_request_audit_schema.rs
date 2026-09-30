//! Real-role coverage for migration 0094's redacted UI request audit stream.
//!
//! The validator should run this against an isolated database with
//! `HEPHAESTUS_POSTGRES_TEST_URL`; this draft intentionally contains no
//! application wiring or production test fixture shortcuts.

use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::env;
use uuid::Uuid;

const EXPECTED_MIGRATION: i64 = 94;

#[tokio::test]
#[serial]
async fn ui_request_audit_is_redacted_append_only_and_worker_owned() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI request audit schema: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect audit schema bootstrap");
    sqlx::migrate!("../../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0094");
    let max_migration: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&bootstrap)
    .await
    .expect("read migration marker")
    .expect("migration marker exists");
    assert!(max_migration >= EXPECTED_MIGRATION);

    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app = role_pool(&database_url, "hephaestus_app").await;
    assert_table_privilege(&bootstrap, "hephaestus_worker", "INSERT", true).await;
    assert_table_privilege(&bootstrap, "hephaestus_worker", "SELECT", false).await;
    assert_table_privilege(&bootstrap, "hephaestus_app", "INSERT", false).await;
    assert_table_privilege(&bootstrap, "hephaestus_app", "SELECT", false).await;

    let event_id = Uuid::new_v4();
    insert_anonymous_denial(&worker, event_id).await;
    let unknown_id = Uuid::new_v4();
    insert_unknown(&worker, unknown_id).await;
    let count_before_rollback: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_request_audit_events WHERE id = $1")
            .bind(event_id)
            .fetch_one(&bootstrap)
            .await
            .expect("read committed anonymous audit event");
    assert_eq!(count_before_rollback, 1);
    let unknown_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_request_audit_events WHERE id = $1")
            .bind(unknown_id)
            .fetch_one(&bootstrap)
            .await
            .expect("read committed unknown audit event");
    assert_eq!(unknown_count, 1);

    for (decision, outcome) in [("undetermined", "succeeded"), ("allowed", "unknown")] {
        let invalid = insert_values(&worker, Uuid::new_v4(), decision, outcome).await;
        assert!(
            invalid.is_err(),
            "invalid audit pair unexpectedly succeeded"
        );
        assert_sqlstate(
            &take_sql_error(invalid, "invalid audit pair unexpectedly succeeded"),
            "23514",
        );
    }

    let constraints: Vec<(String, String)> = sqlx::query_as(
        "SELECT conname::text, pg_get_constraintdef(oid)::text
         FROM pg_catalog.pg_constraint
         WHERE conrelid = 'public.ui_request_audit_events'::regclass
           AND contype = 'c'
           AND conname IN (
             'ui_request_audit_events_decision_check',
             'ui_request_audit_events_outcome_check',
             'ui_request_audit_events_decision_outcome_check'
           )
         ORDER BY conname",
    )
    .fetch_all(&bootstrap)
    .await
    .expect("inspect migration 0094 audit checks");
    assert_eq!(constraints.len(), 3);
    assert!(constraints.iter().any(|(name, definition)| {
        name == "ui_request_audit_events_decision_check" && definition.contains("undetermined")
    }));
    assert!(constraints.iter().any(|(name, definition)| {
        name == "ui_request_audit_events_outcome_check" && definition.contains("unknown")
    }));
    assert!(constraints.iter().any(|(name, definition)| {
        name == "ui_request_audit_events_decision_outcome_check"
            && definition.contains("undetermined")
            && definition.contains("unknown")
    }));

    let rolled_back_id = Uuid::new_v4();
    let mut transaction = worker.begin().await.expect("begin worker rollback test");
    insert_event(&mut transaction, rolled_back_id)
        .await
        .expect("insert event before rollback");
    transaction.rollback().await.expect("rollback audit event");
    let count_after_rollback: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_request_audit_events WHERE id = $1")
            .bind(rolled_back_id)
            .fetch_one(&bootstrap)
            .await
            .expect("read rolled-back audit event");
    assert_eq!(count_after_rollback, 0);

    let app_insert = insert_event_with_pool(&app, Uuid::new_v4()).await;
    assert!(
        app_insert.is_err(),
        "application role must not write audit rows"
    );
    assert_sqlstate(
        &take_sql_error(app_insert, "app insert unexpectedly succeeded"),
        "42501",
    );
    let app_read = sqlx::query("SELECT id FROM ui_request_audit_events LIMIT 1")
        .fetch_optional(&app)
        .await;
    assert!(
        app_read.is_err(),
        "application role has no direct audit read"
    );
    assert_sqlstate(
        &take_sql_error(app_read, "app read unexpectedly succeeded"),
        "42501",
    );

    let update =
        sqlx::query("UPDATE ui_request_audit_events SET reason_code = 'none' WHERE id = $1")
            .bind(event_id)
            .execute(&bootstrap)
            .await;
    assert!(update.is_err(), "audit rows are immutable");
    assert_sqlstate(
        &take_sql_error(update, "audit update unexpectedly succeeded"),
        "23000",
    );
    let delete = sqlx::query("DELETE FROM ui_request_audit_events WHERE id = $1")
        .bind(event_id)
        .execute(&bootstrap)
        .await;
    assert!(delete.is_err(), "audit rows cannot be deleted");
    assert_sqlstate(
        &take_sql_error(delete, "audit delete unexpectedly succeeded"),
        "23000",
    );
}

async fn role_pool(database_url: &str, role: &str) -> PgPool {
    let role = role.to_owned();
    PgPoolOptions::new()
        .max_connections(2)
        .after_connect(move |connection, _metadata| {
            let role = role.clone();
            Box::pin(async move {
                match role.as_str() {
                    "hephaestus_worker" => sqlx::query("SET ROLE hephaestus_worker")
                        .execute(&mut *connection)
                        .await
                        .map(|_| ()),
                    "hephaestus_app" => sqlx::query("SET ROLE hephaestus_app")
                        .execute(&mut *connection)
                        .await
                        .map(|_| ()),
                    _ => panic!("unexpected test role"),
                }
            })
        })
        .connect(database_url)
        .await
        .expect("connect role pool")
}

async fn assert_table_privilege(pool: &PgPool, role: &str, privilege: &str, expected: bool) {
    let actual: bool =
        sqlx::query_scalar("SELECT has_table_privilege($1, 'public.ui_request_audit_events', $2)")
            .bind(role)
            .bind(privilege)
            .fetch_one(pool)
            .await
            .expect("read audit table privilege");
    assert_eq!(
        actual, expected,
        "unexpected {privilege} privilege for {role}"
    );
}

async fn insert_anonymous_denial(pool: &PgPool, event_id: Uuid) {
    sqlx::query(
        "INSERT INTO ui_request_audit_events
            (id, request_id, surface, decision, outcome, reason_code, occurred_at)
         VALUES ($1, $2, 'bootstrap', 'denied', 'not_attempted', 'unauthenticated', statement_timestamp())",
    )
    .bind(event_id)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("insert anonymous denial");
}

async fn insert_event_with_pool(pool: &PgPool, event_id: Uuid) -> Result<(), sqlx::Error> {
    insert_values(pool, event_id, "denied", "not_attempted").await
}

async fn insert_unknown(pool: &PgPool, event_id: Uuid) {
    insert_values(pool, event_id, "undetermined", "unknown")
        .await
        .expect("insert unknown audit event");
}

async fn insert_values(
    pool: &PgPool,
    event_id: Uuid,
    decision: &str,
    outcome: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ui_request_audit_events
            (id, request_id, surface, decision, outcome, reason_code, occurred_at)
         VALUES ($1, $2, 'bootstrap', $3, $4, 'unavailable', statement_timestamp())",
    )
    .bind(event_id)
    .bind(Uuid::new_v4())
    .bind(decision)
    .bind(outcome)
    .execute(pool)
    .await
    .map(|_| ())
}

async fn insert_event(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ui_request_audit_events
            (id, request_id, surface, decision, outcome, reason_code, occurred_at)
         VALUES ($1, $2, 'bootstrap', 'denied', 'not_attempted', 'unauthenticated', statement_timestamp())",
    )
    .bind(event_id)
    .bind(Uuid::new_v4())
    .execute(&mut **transaction)
    .await
    .map(|_| ())
}

fn assert_sqlstate(error: &sqlx::Error, expected: &str) {
    let code = error
        .as_database_error()
        .and_then(|database| database.code().map(std::borrow::Cow::into_owned));
    assert_eq!(code.as_deref(), Some(expected));
}

fn take_sql_error<T>(result: Result<T, sqlx::Error>, message: &str) -> sqlx::Error {
    match result {
        Ok(_) => panic!("{message}"),
        Err(error) => error,
    }
}
