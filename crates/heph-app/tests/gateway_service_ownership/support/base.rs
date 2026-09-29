use super::*;

pub struct Fixture {
    pub gateway: Uuid,
    pub revision: Uuid,
}

pub async fn test_pool() -> Option<sqlx::PgPool> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(16)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway migrations");
    let max_version: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("read latest migration");
    let max_version = max_version.expect("migrations are present");
    assert!(max_version >= 76);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration={max_version}");
    Some(pool)
}

pub async fn worker_ownership() -> PostgresGatewayServiceOwnership {
    let database_url =
        env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("worker ownership test database URL");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'gateway-ownership-test'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect worker PostgreSQL pool");
    PostgresGatewayServiceOwnership::new(pool)
}

pub async fn worker_failure_store() -> PostgresGatewayServiceFailureStore {
    let database_url =
        env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("worker failure test database URL");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'gateway-failure-test'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect worker failure PostgreSQL pool");
    PostgresGatewayServiceFailureStore::new(pool)
}

pub async fn wait_for_lock(pool: &sqlx::PgPool, query_fragment: &str) {
    wait_for_lock_named(pool, "gateway-ownership-test", query_fragment).await;
}

pub async fn wait_for_lock_named(
    pool: &sqlx::PgPool,
    application_name: &str,
    query_fragment: &str,
) {
    let pattern = format!("%{query_fragment}%");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                     SELECT 1 FROM pg_stat_activity
                      WHERE application_name = $2
                        AND state = 'active'
                        AND wait_event_type = 'Lock'
                        AND query LIKE $1
                 )",
            )
            .bind(&pattern)
            .bind(application_name)
            .fetch_one(pool)
            .await
            .expect("inspect PostgreSQL lock wait");
            if waiting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("ownership operation reached expected lock wait");
}
