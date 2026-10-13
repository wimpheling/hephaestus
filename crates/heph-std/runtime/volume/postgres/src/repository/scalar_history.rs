//! Global original history. Released evidence never reads the current generation.
use crate::{
    PostgresVolumeMetadataRepository, ScalarMetadataSchema, errors::metadata, models::LeaseRow,
};
use runtime_types::RunId;
use volume_trait::{ScalarLeaseHistory, VolumeError};

#[derive(sqlx::FromRow)]
struct HistoryRow {
    #[sqlx(flatten)]
    lease: LeaseRow,
    state: String,
    released_at: Option<time::OffsetDateTime>,
    recovering_at: Option<time::OffsetDateTime>,
    exact: bool,
    current: bool,
    resource_state: String,
}

pub async fn observe(
    repository: &PostgresVolumeMetadataRepository,
    run: RunId,
) -> Result<ScalarLeaseHistory, VolumeError> {
    if repository.scalar_schema != ScalarMetadataSchema::Profiled {
        return Err(VolumeError::InvalidState(
            "global scalar history requires explicit profile schema",
        ));
    }
    let mut tx = repository.pool.begin().await.map_err(metadata)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(metadata)?;
    let worker: bool = sqlx::query_scalar("SELECT current_user='hephaestus_worker'")
        .fetch_one(&mut *tx)
        .await
        .map_err(metadata)?;
    if !worker {
        return Err(VolumeError::InvalidState(
            "global scalar history requires worker role",
        ));
    }
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs execution JOIN run_legacy_vm_placements plan ON plan.run_id=execution.id WHERE execution.id=$1 AND legacy_scalar_run_matches(execution) AND legacy_vm_placement_matches(execution,plan)) AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease JOIN run_legacy_vm_placements plan ON plan.run_id=$1 AND plan.producer_kind='update_hook' AND plan.producer_id=lease.update_id WHERE lease.run_id IS DISTINCT FROM $1)")
        .bind(run.as_uuid()).fetch_one(&mut *tx).await.map_err(metadata)?;
    if !valid {
        return Err(VolumeError::InvalidState(
            "global scalar history has foreign or unproven ownership",
        ));
    }
    let rows:Vec<HistoryRow>=sqlx::query_as("SELECT lease.*,legacy_scalar_lease_matches(lease) AS exact,COALESCE(volume.lease_generation=lease.fencing_token AND volume.host_id=lease.host_id AND volume.provisioning_state='ready',false) AS current,volume.state AS resource_state FROM agent_instance_volume_leases lease JOIN agent_instance_state_volumes volume ON volume.id=lease.volume_id WHERE lease.run_id=$1 ORDER BY lease.id")
        .bind(run.as_uuid()).fetch_all(&mut *tx).await.map_err(metadata)?;
    let result = classify(rows)?;
    tx.commit().await.map_err(metadata)?;
    Ok(result)
}
fn classify(mut rows: Vec<HistoryRow>) -> Result<ScalarLeaseHistory, VolumeError> {
    if rows.len() > 1 {
        return Err(VolumeError::InvalidState(
            "multiple original scalar leases remain Held",
        ));
    }
    let Some(row) = rows.pop() else {
        return Ok(ScalarLeaseHistory::NoHistory);
    };
    if !row.exact {
        return Err(VolumeError::InvalidState(
            "scalar original lease tuple contradicts released pins",
        ));
    }
    if row.released_at.is_some() && row.state == "released" {
        return Ok(ScalarLeaseHistory::Released(row.lease.try_into()?));
    }
    if row.released_at.is_none() && row.current {
        if row.state == "active"
            && row.recovering_at.is_none()
            && matches!(row.resource_state.as_str(), "ready" | "attached")
        {
            return Ok(ScalarLeaseHistory::Held(row.lease.try_into()?));
        }
        if row.state == "recovery_required"
            && row.recovering_at.is_some()
            && row.resource_state == "recovering"
        {
            return Ok(ScalarLeaseHistory::HeldRecovering(row.lease.try_into()?));
        }
    }
    Err(VolumeError::InvalidState(
        "scalar held history state or fence contradicts its resource",
    ))
}
