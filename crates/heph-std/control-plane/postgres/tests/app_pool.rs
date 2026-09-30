//! Application pool role isolation against real `PostgreSQL`.

use authz_postgres::begin_actor_transaction;
use control_plane_postgres::connect_app;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;

#[tokio::test]
async fn connect_app_sets_role_on_two_connections_and_denies_worker_paths() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping application pool role test: test URL is unset");
        return;
    };

    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL bootstrap pool");
    sqlx::migrate!("../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations");
    let max_migration: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&bootstrap)
    .await
    .expect("read migration marker")
    .expect("migration marker must exist");
    assert!(
        max_migration >= 94,
        "expected migration 94, got {max_migration}"
    );

    let pool = connect_app(&database_url, 2)
        .await
        .expect("connect application-role pool");
    let mut first = pool.acquire().await.expect("acquire first app connection");
    let mut second = pool.acquire().await.expect("acquire second app connection");
    let (first_user, second_user) = tokio::join!(
        sqlx::query_scalar::<_, String>("SELECT current_user").fetch_one(&mut *first),
        sqlx::query_scalar::<_, String>("SELECT current_user").fetch_one(&mut *second),
    );
    let first_user = first_user.expect("first current_user");
    let second_user = second_user.expect("second current_user");
    assert_eq!(first_user, "hephaestus_app");
    assert_eq!(second_user, "hephaestus_app");
    println!(
        "REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration={max_migration} \
         app_pool_current_users={first_user},{second_user}"
    );
    drop(first);
    drop(second);

    sqlx::query("SELECT id, project_id FROM gateways LIMIT 0")
        .execute(&pool)
        .await
        .expect("application role may inspect the scoped gateway columns");
    let denied_gateway_column = sqlx::query("SELECT name FROM gateways LIMIT 0")
        .execute(&pool)
        .await
        .expect_err("application role must not read ungranted gateway columns");
    let database_error = denied_gateway_column
        .as_database_error()
        .expect("gateway column denial must be a database error");
    assert_eq!(database_error.code().as_deref(), Some("42501"));

    let visible_usage_rows = sqlx::query(
        "SELECT project_id, storage_dropped_chunks, storage_dropped_bytes
           FROM gateway_service_log_project_usage LIMIT 1",
    )
    .fetch_all(&pool)
    .await
    .expect("application role may read actor-scoped project usage columns");
    assert!(
        visible_usage_rows.is_empty(),
        "an actor-less application session must see no project usage rows"
    );
    let denied_worker_columns = sqlx::query(
        "SELECT retained_bytes, retained_chunks, retained_epochs
           FROM gateway_service_log_project_usage LIMIT 0",
    )
    .execute(&pool)
    .await
    .expect_err("application role must not read ungranted retained usage columns");
    let database_error = denied_worker_columns
        .as_database_error()
        .expect("worker table denial must be a database error");
    assert_eq!(database_error.code().as_deref(), Some("42501"));
}

#[tokio::test]
async fn actor_context_is_cleared_when_an_app_pool_connection_is_reused() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping actor context pool test: test URL is unset");
        return;
    };

    let bootstrap = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL bootstrap pool");
    sqlx::migrate!("../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations");

    let pool = connect_app(&database_url, 1)
        .await
        .expect("connect single-connection application-role pool");
    let (identity, expected_actor_id, expected_request_id, expected_occurrence_id) =
        actor_context_fixture();

    let mut transaction = begin_actor_transaction(&pool, &identity)
        .await
        .expect("begin canonical actor transaction");
    let (
        backend_pid,
        current_user,
        actor_id,
        subject_type,
        current_request_id,
        current_occurrence_id,
    ): (
        i32,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT pg_backend_pid(), current_user,
                current_setting('hephaestus.actor_id', true),
                current_setting('hephaestus.subject_type', true),
                current_setting('hephaestus.request_id', true),
                current_setting('hephaestus.occurrence_id', true)",
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("read actor context inside transaction");
    assert_eq!(current_user, "hephaestus_app");
    assert_eq!(actor_id.as_deref(), Some(expected_actor_id.as_str()));
    assert_eq!(subject_type.as_deref(), Some("user"));
    assert_eq!(
        current_request_id.as_deref(),
        Some(expected_request_id.as_str())
    );
    assert_eq!(
        current_occurrence_id.as_deref(),
        Some(expected_occurrence_id.as_str())
    );
    transaction
        .commit()
        .await
        .expect("commit actor transaction");

    assert_context_survives_rollback(&pool, &identity).await;

    let mut connection = pool.acquire().await.expect("reacquire app connection");
    let (reused_backend_pid, actor_id, subject_type, current_request_id, current_occurrence_id): (
        i32,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT pg_backend_pid(),
                NULLIF(current_setting('hephaestus.actor_id', true), ''),
                NULLIF(current_setting('hephaestus.subject_type', true), ''),
                NULLIF(current_setting('hephaestus.request_id', true), ''),
                NULLIF(current_setting('hephaestus.occurrence_id', true), '')",
    )
    .fetch_one(&mut *connection)
    .await
    .expect("read actor context after pool reuse");
    assert_eq!(reused_backend_pid, backend_pid);
    assert!(actor_id.is_none());
    assert!(subject_type.is_none());
    assert!(current_request_id.is_none());
    assert!(current_occurrence_id.is_none());
}

fn actor_context_fixture() -> (AuthenticatedIdentity, String, String, String) {
    let identity = AuthenticatedIdentity::new(
        UserId::new(),
        "https://issuer.example",
        "rls-context-test",
        json!({"email_verified": true}),
        RequestId::new(),
    )
    .with_idempotency_id(RequestId::new());
    let expected_actor_id = identity.user_id.to_string();
    let expected_request_id = identity.request_id.to_string();
    let expected_occurrence_id = identity.idempotency_id.to_string();
    (
        identity,
        expected_actor_id,
        expected_request_id,
        expected_occurrence_id,
    )
}

async fn assert_context_survives_rollback(pool: &sqlx::PgPool, identity: &AuthenticatedIdentity) {
    let mut rolled_back = begin_actor_transaction(pool, identity)
        .await
        .expect("begin second canonical actor transaction");
    let rollback_subject_type: Option<String> =
        sqlx::query_scalar("SELECT current_setting('hephaestus.subject_type', true)")
            .fetch_one(&mut *rolled_back)
            .await
            .expect("read actor context before rollback");
    assert_eq!(rollback_subject_type.as_deref(), Some("user"));
    rolled_back
        .rollback()
        .await
        .expect("rollback actor transaction");
}
