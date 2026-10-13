//! Complete read-only comparison of original instance Drain deliveries.

mod decode;
#[cfg(test)]
mod tests;

use release_service::InstanceExecutionConfiguration;
use run_domain::CancelRun;
use run_orchestrator::RepositoryError;
use runtime_types::{AgentInstanceId, RunId};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::errors::storage;

/// Original instance, claims and prepared target supplied as comparison data.
///
/// These values grant no authority. The native worker snapshot must positively
/// qualify their exact persisted original linkage before returning any row.
#[derive(Debug, Clone, Copy)]
pub struct InstanceDrainOriginal<'a> {
    /// Actual instance belonging to the original prepared Create.
    pub instance_id: AgentInstanceId,
    /// Original recipe Drain attempt, not a fabricated per-Run command.
    pub drain_attempt: Uuid,
    /// Full original persisted Drain claim JSON.
    pub drain_claim: &'a Value,
    /// Original prepared Create attempt.
    pub create_attempt: Uuid,
    /// Full original persisted Create claim JSON.
    pub create_claim: &'a Value,
    /// Original prepared Create target JSON.
    pub prepared: &'a Value,
}

/// One inert row from a fully qualified instance snapshot.
///
/// This is neither delivery authority, a physical receipt nor complete Drain.
/// The original owning controller must requalify every actual Cancel before
/// effects. No command is synthesized when the native result has SQL NULL.
#[derive(Debug, Clone)]
pub struct InstanceDrainRunObservation {
    /// Actual associated Run, including positively qualified completed history.
    pub run_id: RunId,
    /// Exact observed removal Cancel DTO, or no actual per-Run Drain DTO.
    pub cancel: Option<CancelRun>,
    /// Full native original comparison data, preserved without normalization.
    pub original: Value,
}

/// Worker reader retaining one pool and its checked execution configuration.
///
/// Construction proves no physical ownership or permission. The owner must
/// retain and validate its original provider, root and pool custody separately.
pub struct PgInstanceDrainReader {
    worker_pool: PgPool,
    configuration: InstanceExecutionConfiguration,
}

impl PgInstanceDrainReader {
    /// Retains the actual worker pool and complete checked configuration.
    #[must_use]
    pub const fn new(worker_pool: PgPool, configuration: InstanceExecutionConfiguration) -> Self {
        Self {
            worker_pool,
            configuration,
        }
    }

    /// Observes the complete original instance inventory in worker RRRO.
    ///
    /// No row is exposed until the native whole-set qualifier and all decoding
    /// succeed. Empty output is only an observed qualified empty set; it proves
    /// neither earlier-process absence nor physical cleanup or complete Drain.
    /// Every ordinary return awaits rollback; cancellation cannot promise an
    /// awaited rollback from transaction `Drop`.
    ///
    /// # Errors
    /// Rejects unknown, foreign or contradictory origins, role/isolation errors,
    /// malformed rows or transaction failures. The primary error wins over a
    /// rollback failure; a successful result requires successful rollback.
    pub async fn observe(
        &self,
        original: &InstanceDrainOriginal<'_>,
    ) -> Result<Vec<InstanceDrainRunObservation>, RepositoryError> {
        decode::validate_input(original)?;
        let configuration =
            serde_json::to_vec(&self.configuration.canonical_json()).map_err(storage)?;
        let mut transaction = self.worker_pool.begin().await.map_err(storage)?;
        let result = self
            .snapshot(&mut transaction, original, &configuration)
            .await;
        let rollback = transaction.rollback().await.map_err(storage);
        match result {
            Ok(rows) => rollback.map(|()| rows),
            Err(primary) => {
                drop(rollback);
                Err(primary)
            }
        }
    }

    async fn snapshot(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        original: &InstanceDrainOriginal<'_>,
        configuration: &[u8],
    ) -> Result<Vec<InstanceDrainRunObservation>, RepositoryError> {
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut **transaction)
            .await
            .map_err(storage)?;
        sqlx::query("SET LOCAL search_path = pg_catalog, public, pg_temp")
            .execute(&mut **transaction)
            .await
            .map_err(storage)?;
        let settings: (String, String, String) = sqlx::query_as(
            "SELECT current_user::text, current_setting('transaction_isolation'), current_setting('transaction_read_only')",
        ).fetch_one(&mut **transaction).await.map_err(storage)?;
        if settings
            != (
                "hephaestus_worker".into(),
                "repeatable read".into(),
                "on".into(),
            )
        {
            return Err(RepositoryError::InvalidData(
                "instance Drain requires actual worker RRRO",
            ));
        }
        let scope = self.configuration.provider_scope();
        let rows = sqlx::query_as(
            "SELECT run_id,cancel_json,original FROM public.observe_prepared_instance_drain140($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        ).bind(original.instance_id.as_uuid()).bind(original.drain_attempt).bind(original.drain_claim)
         .bind(original.create_attempt).bind(original.create_claim).bind(original.prepared)
         .bind(scope.namespace()).bind(scope.host_id()).bind(configuration)
         .fetch_all(&mut **transaction).await.map_err(storage)?;
        decode::rows(rows, original)
    }
}
