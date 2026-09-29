//! `PostgreSQL` pool and lock-wait helpers for service target tests.

use sqlx::postgres::PgPoolOptions;
use std::{
    env,
    time::{Duration, Instant},
};

pub async fn named_worker_pool(application_name: &str) -> sqlx::PgPool {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("worker test database URL");
    let application_name = application_name.to_owned();
    PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |connection, _metadata| {
            let application_name = application_name.clone();
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SELECT set_config('application_name', $1, false)")
                    .bind(application_name)
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect named worker pool")
}

pub async fn wait_for_blocked_workers(pool: &sqlx::PgPool, holder_pid: i32, names: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let rows = sqlx::query_as::<_, (i32, String, Option<String>, Vec<i32>)>(
            "SELECT pid, application_name, wait_event_type, pg_blocking_pids(pid)
               FROM pg_stat_activity
              WHERE application_name = ANY($1::text[]) AND state <> 'idle'",
        )
        .bind(names.to_vec())
        .fetch_all(pool)
        .await
        .expect("read lock-order waiters");
        let blockers_by_pid = rows
            .iter()
            .map(|(pid, _, _, blockers)| (*pid, blockers.as_slice()))
            .collect::<std::collections::HashMap<_, _>>();
        let blocked_by_holder = |pid: i32| {
            let mut pending = rows
                .iter()
                .find(|(candidate, _, _, _)| *candidate == pid)
                .map_or_else(Vec::new, |(_, _, _, blockers)| blockers.clone());
            let mut visited = std::collections::HashSet::new();
            while let Some(blocker) = pending.pop() {
                if blocker == holder_pid {
                    return true;
                }
                if visited.insert(blocker) {
                    if let Some(next) = blockers_by_pid.get(&blocker) {
                        pending.extend(next.iter().copied());
                    }
                }
            }
            false
        };
        if names.iter().all(|name| {
            rows.iter()
                .any(|(pid, application_name, wait_event_type, _)| {
                    application_name == name
                        && wait_event_type.as_deref() == Some("Lock")
                        && blocked_by_holder(*pid)
                })
        }) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "workers did not block under holder {holder_pid}: {rows:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub async fn test_pool() -> Option<sqlx::PgPool> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(16)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway migrations");
    let max_version: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&pool)
    .await
    .expect("read latest migration")
    .expect("migrations are present");
    assert!(max_version >= 74);
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
                sqlx::query("SET application_name = 'gateway-targets-test'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect worker `PostgreSQL` pool");
    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .expect("read worker current user");
    assert_eq!(current_user, "hephaestus_worker");
    pool
}

// This fixture keeps the complete gateway/release graph in one setup helper so
// each boundary test uses the same valid persisted shape.
