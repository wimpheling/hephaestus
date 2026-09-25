//! Service recovery `PostgreSQL` pools.

use sqlx::postgres::PgPoolOptions;
use std::env;

pub async fn test_pool() -> Option<sqlx::PgPool> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway migrations");
    let max_version: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("read latest migration");
    let max_version = max_version.expect("migrations are present");
    assert!(max_version >= 72);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration={max_version}");
    Some(pool)
}

pub async fn worker_pool() -> sqlx::PgPool {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("worker test database URL");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'gateway-recovery-test'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect worker PostgreSQL pool");
    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .expect("read worker current user");
    assert_eq!(current_user, "hephaestus_worker");
    pool
}
