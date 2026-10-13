use run_domain::{
    RunCleanupHostId, RunCleanupLeaseFence, RunCleanupTarget, RunCleanupVmTarget,
    RunCleanupVmTargetKind,
};
use run_orchestrator::RepositoryError;
use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, RunId};
use serde_json::json;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::VmId;

use super::{
    PgRunCleanupRepository,
    model::{Execution, FenceRow, TargetRow},
};
use crate::{PgRunRepository, errors::storage};

impl PgRunCleanupRepository {
    pub(super) async fn bind_vm(
        &self,
        run_id: RunId,
        instance_id: AgentInstanceId,
        revision_id: AgentInstanceRevisionId,
        scope: &RunCleanupHostId,
        vm_id: &VmId,
    ) -> Result<(), RepositoryError> {
        let mut tx = self.worker_transaction().await?;
        if scope != &self.scope {
            return Err(RepositoryError::InvalidData(
                "configured cleanup scope differs",
            ));
        }
        RunCleanupVmTarget::planned(scope.clone(), vm_id.clone()).map_err(storage)?;
        let run = self.lock_execution(&mut tx, run_id).await?;
        if run.instance_id != instance_id.as_uuid()
            || run.instance_revision_id != revision_id.as_uuid()
            || run.volume_acquisition_closed_at.is_some()
            || !matches!(
                run.state.as_str(),
                "queued" | "leasing_volume" | "provisioning"
            )
        {
            return Err(RepositoryError::InvalidData(
                "exact run is closed to VM planning",
            ));
        }
        if let Some(existing) = run.vm_binding_kind {
            if !matches!(existing.as_str(), "planned" | "persisted")
                || run.vm_id.as_deref() != Some(vm_id.0.as_str())
            {
                return Err(RepositoryError::InvalidData("immutable VM binding differs"));
            }
        } else {
            if run.vm_id.is_some() {
                return Err(RepositoryError::InvalidData(
                    "historical VM ownership is unresolved",
                ));
            }
            let changed=sqlx::query("UPDATE runs SET vm_id=$2,vm_provider_namespace=$3,vm_host_id=$4,vm_binding_kind='planned',updated_at=now() WHERE id=$1 AND vm_id IS NULL AND volume_acquisition_closed_at IS NULL")
                .bind(run_id.as_uuid()).bind(&vm_id.0).bind(scope.provider_namespace()).bind(scope.host_id()).execute(&mut *tx).await.map_err(storage)?;
            if changed.rows_affected() != 1 {
                return Err(RepositoryError::InvalidData(
                    "VM planning changed concurrently",
                ));
            }
            PgRunRepository::append_event_tx(
                &mut tx,
                run_id,
                "run.vm_planned",
                json!({"run_id":run_id}),
                OffsetDateTime::now_utc(),
            )
            .await?;
        }
        tx.commit().await.map_err(storage)
    }

    pub(super) async fn lock_execution(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run_id: RunId,
    ) -> Result<Execution, RepositoryError> {
        // Scope filtering gives unknown/wrong known ownership the same safe
        // error; historical unknown scope is retained only as unresolved data.
        let owner:Option<Uuid>=sqlx::query_scalar("SELECT instance_id FROM runs WHERE id=$1 AND ((vm_provider_namespace=$2 AND vm_host_id=$3) OR (vm_provider_namespace IS NULL AND vm_host_id IS NULL))")
            .bind(run_id.as_uuid()).bind(self.scope.provider_namespace()).bind(self.scope.host_id()).fetch_optional(&mut **tx).await.map_err(storage)?;
        let owner = owner.ok_or(RepositoryError::InvalidData(
            "cleanup run scope is unavailable",
        ))?;
        sqlx::query("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE")
            .bind(owner)
            .execute(&mut **tx)
            .await
            .map_err(storage)?;
        sqlx::query_as("SELECT id,instance_id,instance_revision_id,state,vm_id,vm_provider_namespace,vm_host_id,vm_binding_kind,volume_acquisition_closed_at FROM runs WHERE id=$1 AND instance_id=$2 AND ((vm_provider_namespace=$3 AND vm_host_id=$4) OR (vm_provider_namespace IS NULL AND vm_host_id IS NULL)) FOR UPDATE")
            .bind(run_id.as_uuid()).bind(owner).bind(self.scope.provider_namespace()).bind(self.scope.host_id()).fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(RepositoryError::InvalidData("cleanup run scope is unavailable"))
    }

    pub(super) async fn load_target(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run_id: RunId,
    ) -> Result<Option<RunCleanupTarget>, RepositoryError> {
        let row:Option<TargetRow>=sqlx::query_as("SELECT run_id,instance_id,instance_revision_id,generation,vm_target_kind,vm_provider_namespace,vm_host_id,vm_id,target_digest FROM run_cleanup_targets WHERE run_id=$1 AND ((vm_provider_namespace=$2 AND vm_host_id=$3) OR vm_target_kind='historical_unresolved')")
            .bind(run_id.as_uuid()).bind(self.scope.provider_namespace()).bind(self.scope.host_id()).fetch_optional(&mut **tx).await.map_err(storage)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let rows:Vec<FenceRow>=sqlx::query_as("SELECT run_id,lease_id,volume_id,host_id,fencing_token FROM run_cleanup_target_leases WHERE run_id=$1 AND generation=$2 ORDER BY volume_id")
            .bind(run_id.as_uuid()).bind(row.generation).fetch_all(&mut **tx).await.map_err(storage)?;
        row.target(
            rows.into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
        )
        .map(Some)
    }

    pub(super) async fn lock_current_leases(
        tx: &mut Transaction<'_, Postgres>,
        run_id: RunId,
    ) -> Result<Vec<RunCleanupLeaseFence>, RepositoryError> {
        let resources:Vec<Uuid>=sqlx::query_scalar("SELECT volume_id FROM agent_instance_volume_leases WHERE run_id=$1 AND released_at IS NULL ORDER BY volume_id")
            .bind(run_id.as_uuid()).fetch_all(&mut **tx).await.map_err(storage)?;
        sqlx::query(
            "SELECT id FROM agent_instance_state_volumes WHERE id=ANY($1) ORDER BY id FOR UPDATE",
        )
        .bind(&resources)
        .fetch_all(&mut **tx)
        .await
        .map_err(storage)?;
        let rows:Vec<FenceRow>=sqlx::query_as("SELECT run_id,id AS lease_id,volume_id,host_id,fencing_token FROM agent_instance_volume_leases WHERE run_id=$1 AND released_at IS NULL ORDER BY volume_id,id FOR UPDATE")
            .bind(run_id.as_uuid()).fetch_all(&mut **tx).await.map_err(storage)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub(super) async fn freeze_target(
        &self,
        run_id: RunId,
    ) -> Result<RunCleanupTarget, RepositoryError> {
        let mut tx = self.worker_transaction().await?;
        let queued = self.lock_queued_cleanup(&mut tx, run_id).await?;
        let (qualified, run) = if let Some((queued, run)) = queued {
            (Some(queued), run)
        } else {
            (None, self.lock_execution(&mut tx, run_id).await?)
        };
        let target = self.freeze_target_tx(&mut tx, run_id, run).await?;
        if let Some(queued) = qualified {
            queued.requalify(&mut tx, run_id, &self.scope).await?;
        }
        tx.commit().await.map_err(storage)?;
        Ok(target)
    }

    // The wake caller has already locked/requalified positive consumed ownership;
    // its planned VM cannot enter the unchanged ordinary unplanned-adoption branch.
    pub(crate) async fn freeze_target_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run_id: RunId,
        mut run: Execution,
    ) -> Result<RunCleanupTarget, RepositoryError> {
        if let Some(target) = self.load_target(tx, run_id).await? {
            return Ok(target);
        }
        if run.state == "cleaned_up" {
            return Err(RepositoryError::InvalidData(
                "historical cleaned run has no fabricated receipt",
            ));
        }
        sqlx::query("UPDATE runs SET volume_acquisition_closed_at=COALESCE(volume_acquisition_closed_at,now()) WHERE id=$1")
            .bind(run_id.as_uuid()).execute(&mut **tx).await.map_err(storage)?;
        if run.vm_binding_kind.is_none()
            && run.vm_id.is_none()
            && crate::planned_create::consume_for_cleanup(tx, run_id, &self.scope).await?
        {
            run = self.lock_execution(tx, run_id).await?;
        }
        let leases = Self::lock_current_leases(tx, run_id).await?;
        let target = RunCleanupTarget::new(
            run_id,
            AgentInstanceId::from_uuid(run.instance_id),
            AgentInstanceRevisionId::from_uuid(run.instance_revision_id),
            run.vm_target()?,
            1,
            leases,
        )
        .map_err(storage)?;
        Self::insert_target(tx, &target).await?;
        PgRunRepository::append_event_tx(tx,run_id,"run.cleanup_started",json!({"run_id":run_id,"generation":target.generation(),"lease_count":target.leases().len()}),OffsetDateTime::now_utc()).await?;
        Ok(target)
    }

    async fn insert_target(
        tx: &mut Transaction<'_, Postgres>,
        target: &RunCleanupTarget,
    ) -> Result<(), RepositoryError> {
        let vm = target.vm_target();
        let kind = match vm.kind() {
            RunCleanupVmTargetKind::Persisted => "persisted",
            RunCleanupVmTargetKind::Planned => "planned",
            RunCleanupVmTargetKind::HistoricalUnresolved => "historical_unresolved",
        };
        sqlx::query("INSERT INTO run_cleanup_targets(run_id,instance_id,instance_revision_id,generation,vm_target_kind,vm_provider_namespace,vm_host_id,vm_id,target_digest,lease_count) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
            .bind(target.run_id().as_uuid()).bind(target.instance_id().as_uuid()).bind(target.revision_id().as_uuid()).bind(i64::try_from(target.generation()).map_err(storage)?).bind(kind).bind(vm.host().map(RunCleanupHostId::provider_namespace)).bind(vm.host().map(RunCleanupHostId::host_id)).bind(vm.vm_id().map(|id|id.0.as_str())).bind(target.digest().as_slice()).bind(i32::try_from(target.leases().len()).map_err(storage)?).execute(&mut **tx).await.map_err(storage)?;
        for lease in target.leases() {
            sqlx::query("INSERT INTO run_cleanup_target_leases(run_id,generation,lease_id,volume_id,host_id,fencing_token) VALUES($1,$2,$3,$4,$5,$6)")
                .bind(target.run_id().as_uuid()).bind(i64::try_from(target.generation()).map_err(storage)?).bind(lease.lease_id().as_uuid()).bind(lease.volume_id().as_uuid()).bind(lease.host_id()).bind(lease.fencing_token()).execute(&mut **tx).await.map_err(storage)?;
        }
        Ok(())
    }
}
