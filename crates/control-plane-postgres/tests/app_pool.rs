//! Application pool role isolation against real `PostgreSQL`.

use control_plane_postgres::connect_app;
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
    sqlx::migrate!("../../migrations")
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
        max_migration >= 89,
        "expected migration 89, got {max_migration}"
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
