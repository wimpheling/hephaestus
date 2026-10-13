//! Complete113 early reservations for the explicitly configured119 route.

use runtime_types::RunId;
use sqlx::{Postgres, Transaction};
use volume_trait::VolumeError;

use crate::errors::metadata;

#[derive(sqlx::FromRow)]
struct Reservation {
    lock_class: i32,
    command_mutex: Option<i64>,
    first_mutex: Option<i32>,
    second_mutex: Option<i32>,
}

// Called only in a fresh owning worker transaction before ANY parent/child row.
pub async fn acquire(tx: &mut Transaction<'_, Postgres>, run: RunId) -> Result<(), VolumeError> {
    let rows: Vec<Reservation> = sqlx::query_as(
        "SELECT * FROM invocation119_reservation_context($1) ORDER BY lock_class,command_mutex,first_mutex,second_mutex",
    )
    .bind(run.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(metadata)?;
    if rows.is_empty() {
        return Err(VolumeError::InvalidState(
            "Invocation whole-preparation reservations absent",
        ));
    }
    for row in rows {
        acquire_one(tx, row).await?;
    }
    Ok(())
}

async fn acquire_one(
    tx: &mut Transaction<'_, Postgres>,
    row: Reservation,
) -> Result<(), VolumeError> {
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
                .map_err(metadata)?;
        }
        (1, None, Some(first), Some(second)) => {
            sqlx::query("SELECT pg_advisory_xact_lock($1,$2)")
                .bind(first)
                .bind(second)
                .execute(&mut **tx)
                .await
                .map_err(metadata)?;
        }
        _ => {
            return Err(VolumeError::InvalidState(
                "Invocation reservation tag or key shape differs",
            ));
        }
    }
    Ok(())
}
