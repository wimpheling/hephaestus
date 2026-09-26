//! Focused cancellation reconciliation coverage.

use crate::revoked_raw_run_ids;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

#[tokio::test]
async fn revoked_raw_run_query_is_distinct_and_cancellation_aware() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect control-plane PostgreSQL");
    sqlx::raw_sql(
        "CREATE TEMP TABLE runs (
                 id uuid PRIMARY KEY,
                 state text NOT NULL,
                 cancel_requested_at timestamptz
             );
             CREATE TEMP TABLE secret_runtime_sessions (
                 id uuid PRIMARY KEY,
                 run_id uuid NOT NULL,
                 status text NOT NULL
             );
             CREATE TEMP TABLE secret_leases (
                 id uuid PRIMARY KEY,
                 session_id uuid NOT NULL,
                 delivery_mode text NOT NULL
             );",
    )
    .execute(&pool)
    .await
    .expect("create focused reconciliation tables");
    let run_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    sqlx::query("INSERT INTO runs VALUES ($1, 'running', NULL)")
        .bind(run_id)
        .execute(&pool)
        .await
        .expect("seed running run");
    sqlx::query("INSERT INTO secret_runtime_sessions VALUES ($1, $2, 'revoked')")
        .bind(session_id)
        .bind(run_id)
        .execute(&pool)
        .await
        .expect("seed revoked runtime session");
    for lease_id in [Uuid::new_v4(), Uuid::new_v4()] {
        sqlx::query("INSERT INTO secret_leases VALUES ($1, $2, 'raw')")
            .bind(lease_id)
            .bind(session_id)
            .execute(&pool)
            .await
            .expect("seed duplicate raw lease");
    }
    assert_eq!(revoked_raw_run_ids(&pool).await.unwrap(), vec![run_id]);
    sqlx::query("UPDATE runs SET cancel_requested_at = now() WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .expect("mark run cancelled");
    assert!(revoked_raw_run_ids(&pool).await.unwrap().is_empty());
}
