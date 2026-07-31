//! `PostgreSQL` control-plane repository adapter.
#![allow(missing_docs)] // Legacy application DTOs are re-exported while ports are introduced.
#![allow(
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::needless_pass_by_value
)]
#![allow(clippy::missing_panics_doc)]

pub mod artifact;
pub mod build;
pub mod event;
pub mod instance;
pub mod launch;
pub mod organization;
pub mod project;
pub mod release;
pub mod repository;
pub mod repository_browser;
pub mod run;

pub use run::{load_vm_launch_contract, recoverable_update_hook_run_ids};

/// Opaque database pool supplied by the composition root.
pub type ControlPlanePool = sqlx::PgPool;

/// Opens a `PostgreSQL` pool for the control-plane adapters.
pub async fn connect(
    database_url: &str,
    max_connections: u32,
) -> Result<ControlPlanePool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(database_url)
        .await
}

/// Returns runs whose revoked raw leases require cancellation.
pub async fn revoked_raw_run_ids(pool: &ControlPlanePool) -> Result<Vec<uuid::Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT session.run_id FROM secret_runtime_sessions AS session
         JOIN secret_leases AS lease ON lease.session_id = session.id
         JOIN runs AS run ON run.id = session.run_id
         WHERE session.status = 'revoked' AND lease.delivery_mode = 'raw'
           AND run.state IN ('provisioning', 'starting', 'running')
           AND run.cancel_requested_at IS NULL ORDER BY session.run_id",
    )
    .fetch_all(pool)
    .await
}

/// Checks whether one lifecycle event was persisted.
pub async fn has_run_event(
    pool: &ControlPlanePool,
    run_id: uuid::Uuid,
    event_type: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM run_events WHERE run_id = $1 AND event_type = $2)",
    )
    .bind(run_id)
    .bind(event_type)
    .fetch_one(pool)
    .await
}

/// Verifies migration and authorization dispatcher installation.
pub async fn verify_contract(pool: &ControlPlanePool) -> Result<(Option<i64>, bool), sqlx::Error> {
    let migration = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
        .fetch_one(pool)
        .await?;
    let melange = sqlx::query_scalar(
        "SELECT to_regprocedure('check_permission(text,text,text,text,text)') IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;
    Ok((migration, melange))
}
pub use sqlx::*;
