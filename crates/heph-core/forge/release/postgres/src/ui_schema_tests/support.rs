use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

pub async fn wait_until_blocked(pool: &PgPool, pid: i32, expected_blocker: i32) {
    timeout(Duration::from_secs(5), async {
        loop {
            let blockers: Vec<i32> = sqlx::query_scalar("SELECT pg_blocking_pids($1)")
                .bind(pid)
                .fetch_one(pool)
                .await
                .expect("read blocking PostgreSQL PIDs");
            if blockers.contains(&expected_blocker) {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("transaction reached the expected lock barrier");
}

pub async fn admin_pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await
        .expect("connect PostgreSQL test database")
}
pub async fn role_pool(url: &str, role: &str, actor: Uuid) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await
        .expect("connect RLS test pool");
    sqlx::query("SELECT set_config('role', $1, false)")
        .bind(role)
        .execute(&pool)
        .await
        .expect("set RLS database role");
    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
        .bind(actor.to_string())
        .execute(&pool)
        .await
        .expect("set RLS actor");
    pool
}

pub async fn assert_state(pool: &PgPool, release: Uuid, expected: &str) {
    let state: String = sqlx::query_scalar("SELECT state FROM releases WHERE id = $1")
        .bind(release)
        .fetch_one(pool)
        .await
        .expect("read release state");
    assert_eq!(state, expected);
}

pub fn assert_sqlstate<T>(result: Result<T, sqlx::Error>, expected: &str) {
    let error = match result {
        Ok(_) => panic!("expected PostgreSQL error {expected}"),
        Err(error) => error,
    };
    let code = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(|code| code.to_string());
    assert_eq!(code.as_deref(), Some(expected));
}
