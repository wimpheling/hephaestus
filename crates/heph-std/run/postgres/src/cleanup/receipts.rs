use run_domain::{
    RunCleanupHostId, RunCleanupReceipt, RunCleanupTarget, RunCleanupVmObservation,
    RunCleanupVmObservationKind,
};
use run_orchestrator::RepositoryError;
use sqlx::{Postgres, Transaction};
use vm_trait::VmId;

use super::{PgRunCleanupRepository, model::ReceiptRow};
use crate::errors::storage;

impl PgRunCleanupRepository {
    pub(super) async fn load_receipt(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        target: &RunCleanupTarget,
    ) -> Result<Option<RunCleanupReceipt>, RepositoryError> {
        let row:Option<ReceiptRow>=sqlx::query_as("SELECT observation_kind,vm_provider_namespace,vm_host_id,vm_id,observed_at,target_digest FROM run_cleanup_receipts WHERE run_id=$1 AND generation=$2")
            .bind(target.run_id().as_uuid()).bind(i64::try_from(target.generation()).map_err(storage)?).fetch_optional(&mut **tx).await.map_err(storage)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let scope =
            RunCleanupHostId::new(row.vm_provider_namespace, row.vm_host_id).map_err(storage)?;
        if scope != self.scope || row.target_digest.as_slice() != target.digest().as_slice() {
            return Err(RepositoryError::InvalidData(
                "persisted cleanup receipt scope differs",
            ));
        }
        let observation = match row.observation_kind.as_str() {
            "destroyed" => RunCleanupVmObservation::destroyed(scope, VmId(row.vm_id)),
            "authoritatively_absent" => {
                RunCleanupVmObservation::authoritatively_absent(scope, VmId(row.vm_id))
            }
            _ => {
                return Err(RepositoryError::InvalidData(
                    "cleanup observation is invalid",
                ));
            }
        }
        .map_err(storage)?;
        RunCleanupReceipt::new(target.clone(), observation, row.observed_at)
            .map(Some)
            .map_err(storage)
    }

    pub(super) async fn record_receipt(
        &self,
        receipt: &RunCleanupReceipt,
    ) -> Result<(), RepositoryError> {
        let mut tx = self.worker_transaction().await?;
        if receipt.observation().host() != &self.scope {
            return Err(RepositoryError::InvalidData(
                "configured cleanup observation scope differs",
            ));
        }
        let queued = self
            .lock_queued_cleanup(&mut tx, receipt.target().run_id())
            .await?;
        if queued.is_none() {
            self.lock_execution(&mut tx, receipt.target().run_id())
                .await?;
        }
        let target = self
            .load_target(&mut tx, receipt.target().run_id())
            .await?
            .ok_or(RepositoryError::InvalidData(
                "cleanup target is unavailable",
            ))?;
        receipt.matches_target(&target).map_err(storage)?;
        let kind = match receipt.observation().kind() {
            RunCleanupVmObservationKind::Destroyed => "destroyed",
            RunCleanupVmObservationKind::AuthoritativelyAbsent => "authoritatively_absent",
        };
        sqlx::query("INSERT INTO run_cleanup_receipts(run_id,generation,target_digest,observation_kind,vm_provider_namespace,vm_host_id,vm_id,observed_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(run_id,generation) DO NOTHING")
            .bind(target.run_id().as_uuid()).bind(i64::try_from(target.generation()).map_err(storage)?).bind(target.digest().as_slice()).bind(kind).bind(self.scope.provider_namespace()).bind(self.scope.host_id()).bind(&receipt.observation().vm_id().0).bind(receipt.observed_at()).execute(&mut *tx).await.map_err(storage)?;
        if self.load_receipt(&mut tx, &target).await?.as_ref() != Some(receipt) {
            return Err(RepositoryError::InvalidData(
                "immutable cleanup receipt differs",
            ));
        }
        if let Some((queued, _)) = queued {
            queued
                .requalify(&mut tx, receipt.target().run_id(), &self.scope)
                .await?;
        }
        tx.commit().await.map_err(storage)
    }
}
