use run_domain::{RunCleanupHostId, RunKind, StartRun};
use run_orchestrator::RepositoryError;
use sqlx::{Postgres, Transaction};
use vm_trait::VmId;

use crate::{PgRunRepository, errors::storage};

pub async fn consume_for_cleanup(
    tx: &mut Transaction<'_, Postgres>,
    run_id: runtime_types::RunId,
    scope: &RunCleanupHostId,
) -> Result<bool, RepositoryError> {
    let vm = run_id.to_string();
    let changed=sqlx::query("UPDATE run_vm_plan_admissions admission SET consumed_at=now(),consumption_kind='cleanup',vm_id=$2,vm_provider_namespace=$3,vm_host_id=$4 FROM runs execution WHERE admission.run_id=$1 AND execution.id=admission.run_id AND admission.consumed_at IS NULL AND execution.vm_plan_admission_required AND execution.vm_id IS NULL AND execution.vm_binding_kind IS NULL AND execution.volume_acquisition_closed_at IS NOT NULL AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=execution.id AND released_at IS NULL)")
        .bind(run_id.as_uuid()).bind(&vm).bind(scope.provider_namespace()).bind(scope.host_id())
        .execute(&mut **tx).await.map_err(storage)?;
    if changed.rows_affected() == 0 {
        return Ok(false);
    }
    sqlx::query("UPDATE runs SET vm_id=$2,vm_provider_namespace=$3,vm_host_id=$4,vm_binding_kind='planned',updated_at=now() WHERE id=$1")
        .bind(run_id.as_uuid()).bind(vm).bind(scope.provider_namespace()).bind(scope.host_id())
        .execute(&mut **tx).await.map_err(storage)?;
    PgRunRepository::append_event_tx(
        tx,
        run_id,
        "run.vm_cleanup_planned",
        serde_json::json!({"run_id":run_id}),
        time::OffsetDateTime::now_utc(),
    )
    .await?;
    Ok(true)
}

pub async fn consume(
    tx: &mut Transaction<'_, Postgres>,
    command: &StartRun,
    scope: &RunCleanupHostId,
    vm_id: &VmId,
) -> Result<(), RepositoryError> {
    if command.kind != RunKind::Update {
        return Err(RepositoryError::InvalidData(
            "historical run has no VM planning admission",
        ));
    }
    if vm_id.0 != command.run_id.to_string() {
        return Err(RepositoryError::InvalidData(
            "fresh VM identity must equal run UUID",
        ));
    }
    let consumed=sqlx::query("UPDATE run_vm_plan_admissions SET consumed_at=now(),consumption_kind='provision',vm_id=$5,vm_provider_namespace=$6,vm_host_id=$7 WHERE run_id=$1 AND command_id=$2 AND instance_id=$3 AND instance_revision_id=$4 AND consumed_at IS NULL")
        .bind(command.run_id.as_uuid()).bind(command.command_id.as_uuid()).bind(command.instance_id.as_uuid())
        .bind(command.instance_revision_id.as_uuid()).bind(&vm_id.0).bind(scope.provider_namespace()).bind(scope.host_id())
        .execute(&mut **tx).await.map_err(storage)?;
    if consumed.rows_affected() != 1 {
        return Err(RepositoryError::InvalidData(
            "precreated update has no unconsumed IO admission",
        ));
    }
    let bound=sqlx::query("UPDATE runs SET vm_id=$2,vm_provider_namespace=$3,vm_host_id=$4,vm_binding_kind='planned',updated_at=now() WHERE id=$1 AND state='queued' AND vm_id IS NULL AND vm_binding_kind IS NULL AND volume_acquisition_closed_at IS NULL")
        .bind(command.run_id.as_uuid()).bind(&vm_id.0).bind(scope.provider_namespace()).bind(scope.host_id())
        .execute(&mut **tx).await.map_err(storage)?;
    if bound.rows_affected() != 1 {
        return Err(RepositoryError::InvalidData(
            "precreated update is not closed to prior IO",
        ));
    }
    PgRunRepository::append_event_tx(
        tx,
        command.run_id,
        "run.vm_planned",
        serde_json::json!({"run_id":command.run_id}),
        time::OffsetDateTime::now_utc(),
    )
    .await
}
