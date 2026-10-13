use run_domain::{Run, RunCleanupReceipt};
use run_orchestrator::RepositoryError;
use serde_json::json;
use time::OffsetDateTime;

use super::PgRunCleanupRepository;
use crate::{PgRunRepository, errors::storage, model::RunRow};

impl PgRunCleanupRepository {
    pub(super) async fn finish(&self, receipt: &RunCleanupReceipt) -> Result<Run, RepositoryError> {
        let mut tx = self.worker_transaction().await?;
        if receipt.observation().host() != &self.scope {
            return Err(RepositoryError::InvalidData(
                "configured cleanup observation scope differs",
            ));
        }
        let run_id = receipt.target().run_id();
        let queued = self.lock_queued_cleanup(&mut tx, run_id).await?;
        let (qualified, execution) = if let Some((queued, execution)) = queued {
            (Some(queued), execution)
        } else {
            (None, self.lock_execution(&mut tx, run_id).await?)
        };
        let target =
            self.load_target(&mut tx, run_id)
                .await?
                .ok_or(RepositoryError::InvalidData(
                    "cleanup target is unavailable",
                ))?;
        receipt.matches_target(&target).map_err(storage)?;
        if self.load_receipt(&mut tx, &target).await?.as_ref() != Some(receipt) {
            return Err(RepositoryError::InvalidData(
                "verified cleanup receipt is unavailable",
            ));
        }
        let invocation: bool =
            sqlx::query_scalar("SELECT run_kind='invocation' FROM runs WHERE id=$1")
                .bind(run_id.as_uuid())
                .fetch_one(&mut *tx)
                .await
                .map_err(storage)?;
        if invocation && qualified.is_none() {
            let configuration =
                self.invocation_configuration
                    .as_ref()
                    .ok_or(RepositoryError::InvalidData(
                        "qualified Invocation configuration is absent",
                    ))?;
            if configuration.provider_scope().namespace() != self.scope.provider_namespace()
                || configuration.provider_scope().host_id() != self.scope.host_id()
            {
                return Err(RepositoryError::InvalidData(
                    "Invocation composition differs",
                ));
            }
            let config = serde_json::to_vec(&configuration.canonical_json()).map_err(storage)?;
            let exact: bool = sqlx::query_scalar("SELECT inspect_qualified_invocation117($1) AND EXISTS(SELECT 1 FROM run_invocation_consumptions WHERE run_id=$1 AND config_bytes=$2)")
                .bind(run_id.as_uuid()).bind(config).fetch_one(&mut *tx).await.map_err(storage)?;
            if !exact {
                return Err(RepositoryError::InvalidData(
                    "immutable117 Invocation lineage is absent or conflicting",
                ));
            }
        }
        let completed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM run_cleanup_completions WHERE run_id=$1 AND generation=$2 AND target_digest=$3)")
            .bind(run_id.as_uuid()).bind(i64::try_from(target.generation()).map_err(storage)?).bind(target.digest().as_slice()).fetch_one(&mut *tx).await.map_err(storage)?;
        if !completed {
            if execution.state != "cleaning_up" || execution.volume_acquisition_closed_at.is_none()
            {
                return Err(RepositoryError::InvalidData(
                    "run has not reached closed cleanup",
                ));
            }
            let held = Self::lock_current_leases(&mut tx, run_id).await?;
            if held.as_slice() != target.leases() {
                return Err(RepositoryError::InvalidData(
                    "canonical cleanup lease set changed",
                ));
            }
            Self::release_complete_set(&mut tx, receipt).await?;
            sqlx::query("INSERT INTO run_cleanup_completions(run_id,generation,target_digest) VALUES($1,$2,$3)")
                .bind(run_id.as_uuid()).bind(i64::try_from(target.generation()).map_err(storage)?).bind(target.digest().as_slice()).execute(&mut *tx).await.map_err(storage)?;
            let changed=sqlx::query("UPDATE runs SET state='cleaned_up',updated_at=now(),state_version=state_version+1 WHERE id=$1 AND state='cleaning_up' AND volume_acquisition_closed_at IS NOT NULL")
                .bind(run_id.as_uuid()).execute(&mut *tx).await.map_err(storage)?;
            if changed.rows_affected() != 1 {
                return Err(RepositoryError::InvalidData("closed run cleanup changed"));
            }
            PgRunRepository::append_event_tx(&mut tx,run_id,"run.cleaned_up",json!({"run_id":run_id,"state":"cleaned_up","cleanup_generation":target.generation()}),OffsetDateTime::now_utc()).await?;
        } else if execution.state != "cleaned_up" {
            return Err(RepositoryError::InvalidData(
                "cleanup completion state is inconsistent",
            ));
        }
        let row: RunRow = sqlx::query_as("SELECT * FROM runs WHERE id=$1")
            .bind(run_id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(storage)?;
        let run = if let Some(queued) = qualified {
            // Final CleanedUp strict proof includes the original receipt,
            // completion and full events; no candidate or transient hydration.
            queued.requalify(&mut tx, run_id, &self.scope).await?
        } else if invocation {
            row.into_checked_invocation()?
        } else {
            row.try_into()?
        };
        tx.commit().await.map_err(storage)?;
        Ok(run)
    }

    async fn release_complete_set(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        receipt: &RunCleanupReceipt,
    ) -> Result<(), RepositoryError> {
        for lease in receipt.target().leases() {
            let released=sqlx::query("UPDATE agent_instance_volume_leases SET released_at=now(),state='released' WHERE id=$1 AND run_id=$2 AND volume_id=$3 AND host_id=$4 AND fencing_token=$5 AND released_at IS NULL")
                .bind(lease.lease_id().as_uuid()).bind(lease.run_id().as_uuid()).bind(lease.volume_id().as_uuid()).bind(lease.host_id()).bind(lease.fencing_token()).execute(&mut **tx).await.map_err(storage)?;
            if released.rows_affected() != 1 {
                return Err(RepositoryError::InvalidData("cleanup lease fence changed"));
            }
            let ready=sqlx::query("UPDATE agent_instance_state_volumes SET state='ready',updated_at=now() WHERE id=$1 AND host_id=$2 AND lease_generation=$3 AND provisioning_state='ready' AND state IN ('ready','attached','recovering') AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases held WHERE held.volume_id=$1 AND held.released_at IS NULL)")
                .bind(lease.volume_id().as_uuid()).bind(lease.host_id()).bind(lease.fencing_token()).execute(&mut **tx).await.map_err(storage)?;
            if ready.rows_affected() != 1 {
                return Err(RepositoryError::InvalidData(
                    "cleanup resource generation changed",
                ));
            }
        }
        Ok(())
    }
}
