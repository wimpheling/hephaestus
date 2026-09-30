//! Service acceptance database pools and worker lock probes.

use super::support_types::{ActivityRow, TestPools};
use sqlx::postgres::PgPoolOptions;
use std::{env, time::Duration};

pub async fn test_pool() -> Option<TestPools> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let admin = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&admin)
        .await
        .expect("apply gateway migrations");
    let version: i64 =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE version = 75")
            .fetch_one(&admin)
            .await
            .expect("migration 75");
    assert_eq!(version, 75);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 migration=75");
    let worker = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'gateway-acceptance-worker'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect worker PostgreSQL pool");
    Some(TestPools { admin, worker })
}

pub async fn wait_for_gateway_lock(
    admin: &sqlx::PgPool,
) -> (i32, String, String, String, Option<String>, Option<String>) {
    for _ in 0..200 {
        let waiting: Option<ActivityRow> = sqlx::query_as(
            "SELECT pid, coalesce(application_name, ''), coalesce(state, ''),
                        coalesce(backend_type, ''), wait_event_type, wait_event
                   FROM pg_stat_activity
                  WHERE datname = current_database()
                    AND application_name = 'gateway-acceptance-worker'
                    AND wait_event_type = 'Lock'
                    AND state = 'active'
                  ORDER BY pid
                  LIMIT 1",
        )
        .fetch_optional(admin)
        .await
        .expect("inspect acceptance lock wait");
        if let Some(waiting) = waiting {
            return waiting;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("worker acceptance did not wait on the held gateway lock");
}

pub async fn wait_for_no_gateway_acceptance_sessions(admin: &sqlx::PgPool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let sessions: Vec<ActivityRow> = sqlx::query_as(
            "SELECT pid, coalesce(application_name, ''), coalesce(state, ''),
                        coalesce(backend_type, ''), wait_event_type, wait_event
                   FROM pg_stat_activity
                  WHERE datname = current_database()
                    AND application_name = 'gateway-acceptance-worker'
                  ORDER BY pid",
        )
        .fetch_all(admin)
        .await
        .expect("inspect gateway acceptance sessions");
        if sessions.is_empty() {
            println!("REAL_GATEWAY_ACTIVE_ROUTES_CANCELLATION_SESSIONS=0");
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "gateway acceptance worker sessions remain after pool close: {sessions:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
