//! Actual admitted Control origins and strict ordinary prepared cleanup.

use run_domain::RunCleanupHostId;
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{
    cleanup_wake::qualification_error,
    model::RunRow,
    queued_cancel_delivery::{Reservation, acquire},
};

#[derive(sqlx::FromRow)]
pub struct Context {
    instance_id: Uuid,
    original: Value,
}

impl Context {
    pub async fn discover(
        tx: &mut Transaction<'_, Postgres>,
        run: RunId,
        scope: &RunCleanupHostId,
        config: &[u8],
    ) -> Result<Self, RepositoryError> {
        sqlx::query_as("SELECT * FROM discover_ordinary_queued_cancel_context140($1,$2,$3,$4)")
            .bind(run.as_uuid())
            .bind(scope.provider_namespace())
            .bind(scope.host_id())
            .bind(config)
            .fetch_one(&mut **tx)
            .await
            .map_err(qualification_error)
    }

    pub async fn load_prepared(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run: RunId,
        scope: &RunCleanupHostId,
        config: &[u8],
    ) -> Result<RunRow, RepositoryError> {
        let row: RunRow =
            sqlx::query_as("SELECT * FROM load_ordinary_queued_cancel_prepared140($1,$2,$3,$4,$5)")
                .bind(run.as_uuid())
                .bind(&self.original)
                .bind(scope.provider_namespace())
                .bind(scope.host_id())
                .bind(config)
                .fetch_one(&mut **tx)
                .await
                .map_err(qualification_error)?;
        if row.instance_id() != self.instance_id {
            return Err(RepositoryError::InvalidData(
                "ordinary queued cleanup instance differs",
            ));
        }
        Ok(row)
    }

    pub async fn reserve_and_lock(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run: RunId,
        scope: &RunCleanupHostId,
        config: &[u8],
        mode: &str,
    ) -> Result<(), RepositoryError> {
        let rows: Vec<Reservation> = sqlx::query_as("SELECT * FROM ordinary_queued_cancel_reservations140($1,$2,$3,$4,$5,$6) ORDER BY lock_class,command_mutex,first_mutex,second_mutex")
            .bind(run.as_uuid()).bind(&self.original).bind(scope.provider_namespace())
            .bind(scope.host_id()).bind(config).bind(mode).fetch_all(&mut **tx).await.map_err(qualification_error)?;
        if rows.is_empty() {
            return Err(RepositoryError::InvalidData(
                "ordinary queued cancellation reservations absent",
            ));
        }
        for row in rows {
            acquire(tx, row).await?;
        }
        sqlx::query("SELECT lock_ordinary_queued_cancel140($1,$2,$3,$4,$5,$6)")
            .bind(run.as_uuid())
            .bind(&self.original)
            .bind(scope.provider_namespace())
            .bind(scope.host_id())
            .bind(config)
            .bind(mode)
            .execute(&mut **tx)
            .await
            .map_err(qualification_error)?;
        Ok(())
    }
}
