//! Sorted complete113 reservations; blocking occurs before all parent rows.

use run_domain::RunCleanupHostId;
use run_orchestrator::{RepositoryError, RunCleanupWakeRequest};
use sqlx::{Postgres, Transaction};

use super::{qualification, qualification_error};
use crate::errors::storage;

#[derive(sqlx::FromRow)]
struct Reservation {
    lock_class: i32,
    command_mutex: Option<i64>,
    first_mutex: Option<i32>,
    second_mutex: Option<i32>,
}

pub(super) async fn acquire(
    tx: &mut Transaction<'_, Postgres>,
    request: &RunCleanupWakeRequest,
    scope: &RunCleanupHostId,
    config: Option<&[u8]>,
) -> Result<(), RepositoryError> {
    let rows: Vec<Reservation> = sqlx::query_as(
        "SELECT * FROM run_cleanup_wake_reservations118($1,$2,$3,$4,$5,$6) ORDER BY lock_class,command_mutex,first_mutex,second_mutex",
    )
    .bind(request.run_id().as_uuid())
    .bind(request.instance_id().as_uuid())
    .bind(qualification::original(request))
    .bind(scope.provider_namespace())
    .bind(scope.host_id())
    .bind(config)
    .fetch_all(&mut **tx)
    .await
    .map_err(qualification_error)?;
    acquire_rows(tx, rows).await
}

pub(super) async fn acquire_queued(
    tx: &mut Transaction<'_, Postgres>,
    request: &RunCleanupWakeRequest,
    scope: &RunCleanupHostId,
    config: &[u8],
) -> Result<(), RepositoryError> {
    let rows: Vec<Reservation> = sqlx::query_as(
        "SELECT * FROM queued_run_cleanup_reservations119($1,$2,$3,$4,$5,$6) ORDER BY lock_class,command_mutex,first_mutex,second_mutex",
    )
    .bind(request.run_id().as_uuid())
    .bind(request.instance_id().as_uuid())
    .bind(qualification::original(request))
    .bind(scope.provider_namespace())
    .bind(scope.host_id())
    .bind(config)
    .fetch_all(&mut **tx)
    .await
    .map_err(qualification_error)?;
    acquire_rows(tx, rows).await
}

async fn acquire_rows(
    tx: &mut Transaction<'_, Postgres>,
    rows: Vec<Reservation>,
) -> Result<(), RepositoryError> {
    if rows.is_empty() {
        return Err(RepositoryError::InvalidData(
            "cleanup wake reservations absent",
        ));
    }
    for row in rows {
        match (
            row.lock_class,
            row.command_mutex,
            row.first_mutex,
            row.second_mutex,
        ) {
            (0, Some(command), None, None) => {
                sqlx::query("SELECT pg_advisory_xact_lock($1)")
                    .bind(command)
                    .execute(&mut **tx)
                    .await
                    .map_err(storage)?;
            }
            (1, None, Some(first), Some(second)) => {
                sqlx::query("SELECT pg_advisory_xact_lock($1,$2)")
                    .bind(first)
                    .bind(second)
                    .execute(&mut **tx)
                    .await
                    .map_err(storage)?;
            }
            _ => {
                return Err(RepositoryError::InvalidData(
                    "cleanup wake reservation shape differs",
                ));
            }
        }
    }
    Ok(())
}
