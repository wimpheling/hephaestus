use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};

use crate::errors::storage;

#[derive(sqlx::FromRow)]
struct Reservation {
    #[sqlx(rename = "command_mutex")]
    command: i64,
    #[sqlx(rename = "first_mutex")]
    first: i32,
    #[sqlx(rename = "second_mutex")]
    second: i32,
}

// The caller owns this fresh worker transaction and holds no parent/child locks.
pub async fn acquire(
    tx: &mut Transaction<'_, Postgres>,
    run: RunId,
) -> Result<(), RepositoryError> {
    let rows: Vec<Reservation> = sqlx::query_as(
        "SELECT * FROM invocation117_reservation_context($1) ORDER BY command_mutex",
    )
    .bind(run.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(storage)?;
    let first = rows.first().ok_or(RepositoryError::InvalidData(
        "Invocation reservation is absent",
    ))?;
    if rows.len() > 2
        || rows
            .iter()
            .any(|row| row.first != first.first || row.second != first.second)
    {
        return Err(RepositoryError::InvalidData(
            "Invocation reservation contradicts producer",
        ));
    }
    for row in &rows {
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(row.command)
            .execute(&mut **tx)
            .await
            .map_err(storage)?;
    }
    sqlx::query("SELECT pg_advisory_xact_lock($1,$2)")
        .bind(first.first)
        .bind(first.second)
        .execute(&mut **tx)
        .await
        .map_err(storage)?;
    Ok(())
}
