//! Globally selected target and complete original fences, using existing codecs.

use run_domain::{Run, RunCleanupHostId, RunCleanupVmTargetKind};
use run_orchestrator::{RepositoryError, RunCleanupWakeBinding, RunCleanupWakeRequest};
use sqlx::{Postgres, Transaction};

use crate::{
    cleanup::model::{FenceRow, TargetRow},
    errors::storage,
};

pub(super) async fn binding(
    tx: &mut Transaction<'_, Postgres>,
    request: &RunCleanupWakeRequest,
    scope: &RunCleanupHostId,
    run: &Run,
) -> Result<RunCleanupWakeBinding, RepositoryError> {
    // Unknown/foreign target is never transformed into host-filtered absence.
    let row: TargetRow = sqlx::query_as(
        "SELECT run_id,instance_id,instance_revision_id,generation,vm_target_kind,vm_provider_namespace,vm_host_id,vm_id,target_digest FROM run_cleanup_targets WHERE run_id=$1",
    )
    .bind(request.run_id().as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage)?
    .ok_or(RepositoryError::InvalidData("cleanup wake target is absent"))?;
    let fences: Vec<FenceRow> = sqlx::query_as(
        "SELECT run_id,lease_id,volume_id,host_id,fencing_token FROM run_cleanup_target_leases WHERE run_id=$1 AND generation=$2 ORDER BY volume_id,lease_id",
    )
    .bind(request.run_id().as_uuid())
    .bind(row.generation)
    .fetch_all(&mut **tx)
    .await
    .map_err(storage)?;
    let target = row.target(
        fences
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?,
    )?;
    if run.id != request.run_id()
        || run.instance_id != request.instance_id()
        || run.cancel_requested_at.is_none()
        || target.run_id() != run.id
        || target.instance_id() != run.instance_id
        || target.revision_id() != run.instance_revision_id
        || target.vm_target().kind() != RunCleanupVmTargetKind::Planned
        || target.vm_target().host() != Some(scope)
        || target.vm_target().vm_id().map(|id| id.0.as_str()) != run.vm_id.as_deref()
    {
        return Err(RepositoryError::InvalidData(
            "cleanup wake target contradicts original Run ownership",
        ));
    }
    Ok(RunCleanupWakeBinding::Owned {
        target: Box::new(target),
    })
}
