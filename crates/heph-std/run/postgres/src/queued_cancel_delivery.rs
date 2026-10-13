//! Configured pre117 Cancel delivery; no execution or cleanup provider authority.

use run_domain::{CancelRun, RunCleanupHostId};
use run_orchestrator::RepositoryError;
use sqlx::{Postgres, Transaction};

use crate::{PgRunRepository, cleanup_wake::qualification_error, errors::storage};

#[derive(sqlx::FromRow)]
pub struct Reservation {
    lock_class: i32,
    command_mutex: Option<i64>,
    first_mutex: Option<i32>,
    second_mutex: Option<i32>,
}

impl PgRunRepository {
    // The existing profiled caller owns a fresh transaction, before any row lock.
    // Consumption presence only selects the unchanged positive119 lock path.
    pub(crate) async fn lock_queued_cancel_delivery(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        command: &CancelRun,
    ) -> Result<bool, RepositoryError> {
        let worker: bool = sqlx::query_scalar("SELECT current_user='hephaestus_worker'")
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?;
        if !worker {
            return Err(RepositoryError::InvalidData(
                "queued Cancel delivery requires the configured worker role",
            ));
        }
        let kind: String = sqlx::query_scalar("SELECT run_kind FROM runs WHERE id=$1")
            .bind(command.run_id.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(storage)?
            .ok_or(RepositoryError::NotFound(command.run_id))?;
        if kind != "invocation" {
            return Ok(false);
        }
        let consumed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=$1)",
        )
        .bind(command.run_id.as_uuid())
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
        if consumed {
            return Ok(false);
        }
        let (scope, config) = self.queued_cancel_configuration()?;
        let payload = serde_json::to_value(command).map_err(storage)?;
        let ordinary = crate::ordinary_queued_cancel::selected(tx, command.run_id).await?;
        let rows: Vec<Reservation> = sqlx::query_as(
            if ordinary {
                "SELECT * FROM ordinary_cancel_delivery_reservations140($1,$2,$3,$4,$5) ORDER BY lock_class,command_mutex,first_mutex,second_mutex"
            } else {
                "SELECT * FROM queued_cancel_reservations119($1,$2,$3,$4,$5) ORDER BY lock_class,command_mutex,first_mutex,second_mutex"
            },
        )
        .bind(command.run_id.as_uuid())
        .bind(&payload)
        .bind(scope.provider_namespace())
        .bind(scope.host_id())
        .bind(&config)
        .fetch_all(&mut **tx)
        .await
        .map_err(qualification_error)?;
        if rows.is_empty() {
            return Err(RepositoryError::InvalidData(
                "queued Cancel delivery reservations absent",
            ));
        }
        // These are the only waits, before parent/late locks. SQL then uses
        // TRY/NOWAIT and requalifies under the required read-committed boundary.
        for row in rows {
            acquire(tx, row).await?;
        }
        sqlx::query(if ordinary {
            "SELECT lock_ordinary_cancel_delivery140($1,$2,$3,$4,$5)"
        } else {
            "SELECT lock_queued_cancel_delivery119($1,$2,$3,$4,$5)"
        })
        .bind(command.run_id.as_uuid())
        .bind(payload)
        .bind(scope.provider_namespace())
        .bind(scope.host_id())
        .bind(config)
        .execute(&mut **tx)
        .await
        .map_err(qualification_error)?;
        Ok(true)
    }

    pub(crate) async fn verify_queued_cancel_delivery(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        command: &CancelRun,
    ) -> Result<(), RepositoryError> {
        let (scope, config) = self.queued_cancel_configuration()?;
        let payload = serde_json::to_value(command).map_err(storage)?;
        let ordinary = crate::ordinary_queued_cancel::selected(tx, command.run_id).await?;
        sqlx::query(if ordinary {
            "SELECT verify_ordinary_cancel_delivery140($1,$2,$3,$4,$5)"
        } else {
            "SELECT verify_queued_cancel_delivery119($1,$2,$3,$4,$5)"
        })
        .bind(command.run_id.as_uuid())
        .bind(payload)
        .bind(scope.provider_namespace())
        .bind(scope.host_id())
        .bind(config)
        .execute(&mut **tx)
        .await
        .map_err(qualification_error)?;
        Ok(())
    }

    fn queued_cancel_configuration(&self) -> Result<(&RunCleanupHostId, Vec<u8>), RepositoryError> {
        let configuration = self.invocation_configuration()?;
        let scope = self.vm_scope.as_ref().ok_or(RepositoryError::InvalidData(
            "queued Cancel delivery requires actual configured owner",
        ))?;
        if self.cleanup_wake_scope.as_ref() != Some(scope) {
            return Err(RepositoryError::InvalidData(
                "queued Cancel delivery requires matching cleanup wake composition",
            ));
        }
        let config = serde_json::to_vec(&configuration.canonical_json()).map_err(storage)?;
        Ok((scope, config))
    }
}

pub async fn acquire(
    tx: &mut Transaction<'_, Postgres>,
    row: Reservation,
) -> Result<(), RepositoryError> {
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
                "queued Cancel delivery reservation shape differs",
            ));
        }
    }
    Ok(())
}
