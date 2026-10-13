//! Complete-set transactions; no filesystem or VM effects occur here.

use runtime_types::{LeaseId, RunId, VolumeId};
use sqlx::{PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use volume_domain::{RunVolumeSelections, VolumeSelectionOrigin};
use volume_trait::{RunVolumeLease, VolumeError};

use super::{catalog, lease_rows};
use crate::{
    errors::{assert_host, metadata},
    models::VolumeRow,
};

pub async fn acquire(
    pool: &PgPool,
    selections: &RunVolumeSelections,
    host: &str,
    now: OffsetDateTime,
    expires: OffsetDateTime,
) -> Result<Vec<RunVolumeLease>, VolumeError> {
    timestamps(host, now, expires)?;
    let run_id = selections.identity().run_id();
    let mut tx = pool.begin().await.map_err(metadata)?;
    catalog::check(&mut tx, selections).await?;
    let acquiring: bool = sqlx::query_scalar(
        "SELECT state IN ('leasing_volume','provisioning') FROM runs WHERE id=$1",
    )
    .bind(run_id.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .map_err(metadata)?;
    if !acquiring {
        return Err(VolumeError::InvalidState("run is not acquiring resources"));
    }
    let volumes = lock_volumes(&mut tx, selections, host).await?;
    let existing = lease_rows::load(&mut tx, run_id, host).await?;
    if !existing.is_empty() {
        exact_set(selections, &existing)?;
        reusable(&mut tx, run_id).await?;
        freeze(&mut tx, selections).await?;
        tx.commit().await.map_err(metadata)?;
        return Ok(existing);
    }
    let frozen: bool =
        sqlx::query_scalar("SELECT volume_selection_snapshot IS NOT NULL FROM runs WHERE id=$1")
            .bind(run_id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(metadata)?;
    if frozen && !selections.selections().is_empty() {
        return Err(VolumeError::InvalidState(
            "frozen acquisition lost its lease evidence",
        ));
    }
    // Validate every resource and every conflicting lease before the first insert.
    for volume in &volumes {
        if volume.state != "ready" || volume.provisioning_state != "ready" {
            return Err(VolumeError::InvalidState(
                "exact acquisition requires ready resources",
            ));
        }
        let holder:Option<uuid::Uuid>=sqlx::query_scalar("SELECT run_id FROM agent_instance_volume_leases WHERE volume_id=$1 AND released_at IS NULL")
            .bind(volume.id).fetch_optional(&mut *tx).await.map_err(metadata)?;
        if let Some(holder) = holder {
            return Err(VolumeError::LeaseConflict {
                volume_id: VolumeId::from_uuid(volume.id),
                holder_run_id: RunId::from_uuid(holder),
            });
        }
    }
    freeze(&mut tx, selections).await?;
    for volume in volumes {
        let selected = selections
            .selections()
            .iter()
            .find(|item| item.scope().volume_id().as_uuid() == volume.id)
            .ok_or(VolumeError::InvalidState("selected resource absent"))?;
        let identity = selected.identity();
        let scope = selected.scope();
        let declaration = selected.declaration();
        let origin = match selected.origin() {
            VolumeSelectionOrigin::Explicit => "explicit",
            VolumeSelectionOrigin::LegacyDeclaration => "legacy_declaration",
            VolumeSelectionOrigin::LegacyOrigin => "legacy_origin",
        };
        let fence = volume
            .lease_generation
            .checked_add(1)
            .ok_or(VolumeError::InvalidState("lease generation overflow"))?;
        sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,acquired_at,heartbeat_at,expires_at,selection_origin,selected_revision_id,selected_release_id,selected_release_agent_id,selected_project_id,slot_key,access_mode,guest_path,slot_required,minimum_capacity_bytes,release_contract_hash) VALUES($1,$2,$3,$4,$5,$6,'active',$7,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)")
            .bind(LeaseId::new().as_uuid()).bind(volume.id).bind(identity.instance_id().as_uuid()).bind(run_id.as_uuid()).bind(host).bind(fence).bind(now).bind(expires).bind(origin)
            .bind(identity.revision_id().as_uuid()).bind(identity.release_id().as_uuid()).bind(identity.release_agent_id().as_uuid()).bind(identity.project_id()).bind(scope.slot().as_str()).bind(scope.access_mode().as_str()).bind(declaration.guest_path().as_str()).bind(declaration.required()).bind(i64::try_from(declaration.minimum_capacity_bytes()).map_err(metadata)?).bind(scope.release_contract_hash().as_bytes().as_slice())
            .execute(&mut *tx).await.map_err(metadata)?;
    }
    let result = lease_rows::load(&mut tx, run_id, host).await?;
    exact_set(selections, &result)?;
    tx.commit().await.map_err(metadata)?;
    Ok(result)
}

/// Complete-set attachment or heartbeat, separate from qualified scalar SQL.
#[derive(Clone, Copy)]
pub enum Refresh {
    Attached,
    Heartbeat,
}

pub async fn refresh(
    pool: &PgPool,
    selections: &RunVolumeSelections,
    host: &str,
    now: OffsetDateTime,
    expires: OffsetDateTime,
    refresh: Refresh,
) -> Result<Vec<RunVolumeLease>, VolumeError> {
    timestamps(host, now, expires)?;
    let run_id = selections.identity().run_id();
    let mut tx = pool.begin().await.map_err(metadata)?;
    catalog::check(&mut tx, selections).await?;
    lock_volumes(&mut tx, selections, host).await?;
    let existing = lease_rows::load(&mut tx, run_id, host).await?;
    exact_set(selections, &existing)?;
    let attached = match refresh {
        Refresh::Attached => true,
        Refresh::Heartbeat => false,
    };
    reusable(&mut tx, run_id).await?;
    let frozen: bool =
        sqlx::query_scalar("SELECT volume_selection_snapshot IS NOT NULL FROM runs WHERE id=$1")
            .bind(run_id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(metadata)?;
    if !frozen {
        return Err(VolumeError::InvalidState(
            "run acquisition is not durably frozen",
        ));
    }
    for evidence in &existing {
        let lease = evidence.lease();
        let updated=sqlx::query("UPDATE agent_instance_volume_leases SET attached_at=CASE WHEN $6 THEN COALESCE(attached_at,$4) ELSE attached_at END,heartbeat_at=$4,expires_at=$5 WHERE id=$1 AND volume_id=$2 AND fencing_token=$3 AND released_at IS NULL AND recovering_at IS NULL")
            .bind(lease.id.as_uuid()).bind(lease.volume_id.as_uuid()).bind(lease.fencing_token).bind(now).bind(expires).bind(attached).execute(&mut *tx).await.map_err(metadata)?;
        if updated.rows_affected() != 1 {
            return Err(VolumeError::StaleLease);
        }
        if attached {
            let updated=sqlx::query("UPDATE agent_instance_state_volumes SET state='attached',updated_at=$3 WHERE id=$1 AND lease_generation=$2 AND state IN ('ready','attached') AND provisioning_state='ready'")
                .bind(lease.volume_id.as_uuid()).bind(lease.fencing_token).bind(now).execute(&mut *tx).await.map_err(metadata)?;
            if updated.rows_affected() != 1 {
                return Err(VolumeError::StaleLease);
            }
        }
    }
    let result = lease_rows::load(&mut tx, run_id, host).await?;
    tx.commit().await.map_err(metadata)?;
    Ok(result)
}

async fn lock_volumes(
    tx: &mut Transaction<'_, Postgres>,
    selections: &RunVolumeSelections,
    host: &str,
) -> Result<Vec<VolumeRow>, VolumeError> {
    let vm_host: Option<String> = sqlx::query_scalar("SELECT vm_host_id FROM runs WHERE id=$1")
        .bind(selections.identity().run_id().as_uuid())
        .fetch_one(&mut **tx)
        .await
        .map_err(metadata)?;
    if let Some(owner) = vm_host.as_deref() {
        assert_host(owner, host)?;
    }
    let ids = selections
        .selections()
        .iter()
        .map(|item| item.scope().volume_id().as_uuid())
        .collect::<Vec<_>>();
    let rows: Vec<VolumeRow> = sqlx::query_as(
        "SELECT * FROM agent_instance_state_volumes WHERE id=ANY($1) ORDER BY id FOR UPDATE",
    )
    .bind(&ids)
    .fetch_all(&mut **tx)
    .await
    .map_err(metadata)?;
    if rows.len() != ids.len() {
        return Err(VolumeError::InvalidState("selected resources absent"));
    }
    for row in &rows {
        assert_host(
            row.host_id
                .as_deref()
                .ok_or(VolumeError::InvalidState("resource host unknown"))?,
            host,
        )?;
        let item = selections
            .selections()
            .iter()
            .find(|item| item.scope().volume_id().as_uuid() == row.id)
            .ok_or(VolumeError::InvalidState("unselected resource"))?;
        if row.project_id != selections.identity().project_id()
            || u64::try_from(row.capacity_bytes)
                .ok()
                .is_none_or(|capacity| capacity < item.declaration().minimum_capacity_bytes())
        {
            return Err(VolumeError::InvalidState(
                "resource project or capacity differs",
            ));
        }
    }
    Ok(rows)
}

fn exact_set(
    selections: &RunVolumeSelections,
    leases: &[RunVolumeLease],
) -> Result<(), VolumeError> {
    if selections.selections().len() != leases.len()
        || selections
            .selections()
            .iter()
            .zip(leases)
            .any(|(item, lease)| lease.selection() != Some(item))
    {
        return Err(VolumeError::InvalidState(
            "canonical lease set differs from exact selections",
        ));
    }
    Ok(())
}

async fn freeze(
    tx: &mut Transaction<'_, Postgres>,
    selections: &RunVolumeSelections,
) -> Result<(), VolumeError> {
    let snapshot = serde_json::to_value(selections).map_err(metadata)?;
    sqlx::query("UPDATE runs SET volume_selection_snapshot=$2 WHERE id=$1 AND volume_selection_snapshot IS NULL")
        .bind(selections.identity().run_id().as_uuid()).bind(snapshot).execute(&mut **tx).await.map_err(metadata)?;
    Ok(())
}

async fn reusable(tx: &mut Transaction<'_, Postgres>, run_id: RunId) -> Result<(), VolumeError> {
    let uncertain:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_instance_volume_leases lease JOIN agent_instance_state_volumes volume ON volume.id=lease.volume_id WHERE lease.run_id=$1 AND lease.released_at IS NULL AND (lease.recovering_at IS NOT NULL OR volume.state NOT IN ('ready','attached') OR volume.provisioning_state<>'ready' OR volume.lease_generation<>lease.fencing_token))")
        .bind(run_id.as_uuid()).fetch_one(&mut **tx).await.map_err(metadata)?;
    if uncertain {
        return Err(VolumeError::StaleLease);
    }
    Ok(())
}

fn timestamps(host: &str, now: OffsetDateTime, expires: OffsetDateTime) -> Result<(), VolumeError> {
    if host.is_empty() || host.len() > 256 || expires <= now {
        return Err(VolumeError::InvalidState("invalid host or lease interval"));
    }
    Ok(())
}
