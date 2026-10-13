//! Scalar compatibility is restricted to proven legacy origin, never a named slot.
//! These queries require private 0108 and are not a 0107-compatible publication.

use runtime_types::{RunId, VolumeId};
use time::OffsetDateTime;
use volume_domain::{RunVolumeSelections, VolumeSelectionOrigin};
use volume_trait::{RunVolumeMetadataRepository, VolumeError, VolumeLease};

use crate::{PostgresVolumeMetadataRepository, errors::metadata, run_volumes::mutations};

fn singleton(selections: &RunVolumeSelections, volume: VolumeId) -> Result<(), VolumeError> {
    match selections.selections() {
        [selected]
            if selected.origin() == VolumeSelectionOrigin::LegacyOrigin
                && selected.scope().volume_id() == volume =>
        {
            Ok(())
        }
        _ => Err(VolumeError::InvalidState(
            "scalar API requires exact single legacy origin",
        )),
    }
}

pub async fn acquire(
    repository: &PostgresVolumeMetadataRepository,
    volume: VolumeId,
    run: RunId,
    host: &str,
    now: OffsetDateTime,
    expires: OffsetDateTime,
) -> Result<VolumeLease, VolumeError> {
    let selections = repository.load_run_selections(run).await?;
    singleton(&selections, volume)?;
    // acquire_run revalidates the exact catalog under consumer/run locks before
    // any resource/lease mutation, and advances the fence through the trigger.
    let leases = repository
        .acquire_run(&selections, host, now, expires)
        .await?;
    let [lease] = leases.as_slice() else {
        return Err(VolumeError::StaleLease);
    };
    Ok(lease.lease().clone())
}

pub async fn refresh(
    repository: &PostgresVolumeMetadataRepository,
    lease: &VolumeLease,
    now: OffsetDateTime,
    expires: OffsetDateTime,
    attached: bool,
) -> Result<VolumeLease, VolumeError> {
    let selections = repository.load_run_selections(lease.run_id).await?;
    singleton(&selections, lease.volume_id)?;
    let result = mutations::refresh(
        &repository.pool,
        &selections,
        &lease.host_id,
        now,
        expires,
        mutations::Refresh::Legacy {
            expected: lease,
            attached,
        },
    )
    .await?;
    let [current] = result.as_slice() else {
        return Err(VolumeError::StaleLease);
    };
    Ok(current.lease().clone())
}

pub async fn active(
    repository: &PostgresVolumeMetadataRepository,
    run: RunId,
    host: &str,
) -> Result<Option<VolumeLease>, VolumeError> {
    let rows = repository.leases_for_run(run, host).await?;
    match rows.as_slice() {
        [] => Ok(None),
        [lease]
            if lease
                .selection()
                .is_some_and(|item| item.origin() == VolumeSelectionOrigin::LegacyOrigin) =>
        {
            Ok(Some(lease.lease().clone()))
        }
        _ => Err(VolumeError::InvalidState(
            "scalar inspection cannot project named or unproven leases",
        )),
    }
}

pub async fn completed(
    repository: &PostgresVolumeMetadataRepository,
    lease: &VolumeLease,
) -> Result<(), VolumeError> {
    // Immutable completion/target/receipt rows prove an already released tuple.
    // Do not compare current resource generation: it may have been safely reused.
    let completed: bool = sqlx::query_scalar("SELECT current_user='hephaestus_worker' AND EXISTS(SELECT 1 FROM agent_instance_volume_leases lease JOIN runs execution ON execution.id=lease.run_id JOIN run_cleanup_target_leases item ON item.run_id=lease.run_id AND item.lease_id=lease.id JOIN run_cleanup_completions completion ON completion.run_id=item.run_id AND completion.generation=item.generation JOIN run_cleanup_targets target ON target.run_id=completion.run_id AND target.generation=completion.generation AND target.target_digest=completion.target_digest WHERE lease.id=$1 AND lease.volume_id=$2 AND lease.run_id=$3 AND lease.host_id=$4 AND lease.fencing_token=$5 AND lease.selection_origin='legacy_origin' AND lease.state='released' AND lease.released_at IS NOT NULL AND execution.state='cleaned_up' AND item.volume_id=lease.volume_id AND item.host_id=lease.host_id AND item.fencing_token=lease.fencing_token AND target.vm_host_id=lease.host_id)")
        .bind(lease.id.as_uuid()).bind(lease.volume_id.as_uuid()).bind(lease.run_id.as_uuid()).bind(&lease.host_id).bind(lease.fencing_token)
        .fetch_one(&repository.pool).await.map_err(metadata)?;
    if !completed {
        return Err(VolumeError::StaleLease);
    }
    Ok(())
}

/// Recovery marks uncertainty under the same lock order; it never proves detach.
pub async fn recover(
    repository: &PostgresVolumeMetadataRepository,
    lease: &VolumeLease,
) -> Result<(), VolumeError> {
    let mut tx = repository.pool.begin().await.map_err(metadata)?;
    crate::run_volumes::lock_consumer_run(&mut tx, lease.run_id).await?;
    let rows = crate::run_volumes::load_leases(&mut tx, lease.run_id, &lease.host_id).await?;
    let [current] = rows.as_slice() else {
        return Err(VolumeError::StaleLease);
    };
    let actual = current.lease();
    if current
        .selection()
        .is_some_and(|item| item.origin() != VolumeSelectionOrigin::LegacyOrigin)
        || actual.id != lease.id
        || actual.volume_id != lease.volume_id
        || actual.run_id != lease.run_id
        || actual.host_id != lease.host_id
        || actual.fencing_token != lease.fencing_token
    {
        return Err(VolumeError::StaleLease);
    }
    let updated=sqlx::query("WITH held AS (UPDATE agent_instance_volume_leases SET recovering_at=COALESCE(recovering_at,now()),state='recovery_required' WHERE id=$1 AND volume_id=$2 AND fencing_token=$3 AND released_at IS NULL AND expires_at<=now() RETURNING volume_id,fencing_token) UPDATE agent_instance_state_volumes volume SET state='recovering',updated_at=now() FROM held WHERE volume.id=held.volume_id AND volume.lease_generation=held.fencing_token")
        .bind(lease.id.as_uuid()).bind(lease.volume_id.as_uuid()).bind(lease.fencing_token).execute(&mut *tx).await.map_err(metadata)?;
    if updated.rows_affected() != 1 {
        return Err(VolumeError::StaleLease);
    }
    tx.commit().await.map_err(metadata)
}
