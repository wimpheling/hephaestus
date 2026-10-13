//! Hydrate all canonical evidence; host mismatch must never look like an empty set.

use capability_domain::{AuthorityHash, CapabilitySlotKey};
use runtime_types::{
    AgentInstanceId, AgentInstanceRevisionId, LeaseId, ReleaseAgentId, ReleaseId, RunId, VolumeId,
};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;
use volume_domain::{
    GuestMountPath, RunVolumeIdentity, VolumeSelectionOrigin, VolumeSlotDeclaration,
};
use volume_trait::{RunVolumeLease, VolumeError, VolumeLease};

use super::catalog;
use crate::errors::{assert_host, metadata};

#[derive(sqlx::FromRow)]
struct SelectedLeaseRow {
    id: Uuid,
    volume_id: Uuid,
    instance_id: Uuid,
    run_id: Uuid,
    host_id: String,
    fencing_token: i64,
    acquired_at: OffsetDateTime,
    heartbeat_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    attached_at: Option<OffsetDateTime>,
    selection_origin: String,
    selected_revision_id: Option<Uuid>,
    selected_release_id: Option<Uuid>,
    selected_release_agent_id: Option<Uuid>,
    selected_project_id: Option<Uuid>,
    slot_key: Option<String>,
    access_mode: Option<String>,
    guest_path: Option<String>,
    slot_required: Option<bool>,
    minimum_capacity_bytes: Option<i64>,
    release_contract_hash: Option<Vec<u8>>,
}

pub async fn lock_consumer_run(
    tx: &mut Transaction<'_, Postgres>,
    run_id: RunId,
) -> Result<(), VolumeError> {
    let worker: bool = sqlx::query_scalar("SELECT current_user='hephaestus_worker'")
        .fetch_one(&mut **tx)
        .await
        .map_err(metadata)?;
    if !worker {
        return Err(VolumeError::InvalidState(
            "exact run metadata requires the worker role",
        ));
    }
    let instance: Uuid = sqlx::query_scalar("SELECT instance_id FROM runs WHERE id=$1")
        .bind(run_id.as_uuid())
        .fetch_optional(&mut **tx)
        .await
        .map_err(metadata)?
        .ok_or(VolumeError::InvalidState("run consumer absent"))?;
    let locked: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE")
            .bind(instance)
            .fetch_optional(&mut **tx)
            .await
            .map_err(metadata)?;
    if locked.is_none() {
        return Err(VolumeError::InvalidState("run consumer absent"));
    }
    sqlx::query("SELECT id FROM runs WHERE id=$1 AND instance_id=$2 FOR UPDATE")
        .bind(run_id.as_uuid())
        .bind(instance)
        .fetch_one(&mut **tx)
        .await
        .map_err(metadata)?;
    Ok(())
}

pub async fn load(
    tx: &mut Transaction<'_, Postgres>,
    run_id: RunId,
    host: &str,
) -> Result<Vec<RunVolumeLease>, VolumeError> {
    load_mode(tx, run_id, host, false).await
}

pub(super) async fn load_nowait(
    tx: &mut Transaction<'_, Postgres>,
    run_id: RunId,
    host: &str,
) -> Result<Vec<RunVolumeLease>, VolumeError> {
    load_mode(tx, run_id, host, true).await
}

async fn load_mode(
    tx: &mut Transaction<'_, Postgres>,
    run_id: RunId,
    host: &str,
    nowait: bool,
) -> Result<Vec<RunVolumeLease>, VolumeError> {
    // All qualified119 early and parent locks are already held. Ordinary
    // consumer-first paths retain their exact blocking query shape.
    let rows: Vec<SelectedLeaseRow> = if nowait {
        sqlx::query("SELECT volume.id FROM agent_instance_state_volumes volume WHERE EXISTS(SELECT 1 FROM agent_instance_volume_leases lease WHERE lease.volume_id=volume.id AND lease.run_id=$1 AND lease.released_at IS NULL) ORDER BY volume.id FOR UPDATE NOWAIT")
        .bind(run_id.as_uuid()).fetch_all(&mut **tx).await.map_err(metadata)?;
        sqlx::query_as(
        "SELECT * FROM agent_instance_volume_leases WHERE run_id=$1 AND released_at IS NULL ORDER BY volume_id,id FOR UPDATE NOWAIT",
    ).bind(run_id.as_uuid()).fetch_all(&mut **tx).await.map_err(metadata)?
    } else {
        sqlx::query("SELECT volume.id FROM agent_instance_state_volumes volume WHERE EXISTS(SELECT 1 FROM agent_instance_volume_leases lease WHERE lease.volume_id=volume.id AND lease.run_id=$1 AND lease.released_at IS NULL) ORDER BY volume.id FOR UPDATE")
        .bind(run_id.as_uuid()).fetch_all(&mut **tx).await.map_err(metadata)?;
        sqlx::query_as(
        "SELECT * FROM agent_instance_volume_leases WHERE run_id=$1 AND released_at IS NULL ORDER BY volume_id,id FOR UPDATE",
    ).bind(run_id.as_uuid()).fetch_all(&mut **tx).await.map_err(metadata)?
    };
    if rows.len() > volume_domain::MAX_VOLUME_SLOTS {
        return Err(VolumeError::InvalidState(
            "canonical run lease set exceeds the bound",
        ));
    }
    let mut leases = rows
        .into_iter()
        .map(|row| {
            assert_host(&row.host_id, host)?;
            hydrate(row)
        })
        .collect::<Result<Vec<_>, VolumeError>>()?;
    leases.sort_by(|left, right| {
        left.selection()
            .map(|item| item.scope().slot())
            .cmp(&right.selection().map(|item| item.scope().slot()))
            .then_with(|| left.lease().volume_id.cmp(&right.lease().volume_id))
            .then_with(|| left.lease().id.cmp(&right.lease().id))
    });
    Ok(leases)
}

fn required<T>(value: Option<T>) -> Result<T, VolumeError> {
    value.ok_or(VolumeError::InvalidState(
        "canonical lease selection evidence is incomplete",
    ))
}

fn hydrate(row: SelectedLeaseRow) -> Result<RunVolumeLease, VolumeError> {
    let lease = VolumeLease {
        id: LeaseId::from_uuid(row.id),
        volume_id: VolumeId::from_uuid(row.volume_id),
        run_id: RunId::from_uuid(row.run_id),
        host_id: row.host_id,
        fencing_token: row.fencing_token,
        acquired_at: row.acquired_at,
        heartbeat_at: row.heartbeat_at,
        expires_at: row.expires_at,
        attached_at: row.attached_at,
    };
    if row.selection_origin == "historical_unproven" {
        if row.selected_revision_id.is_some()
            || row.selected_release_id.is_some()
            || row.selected_release_agent_id.is_some()
            || row.selected_project_id.is_some()
            || row.slot_key.is_some()
            || row.access_mode.is_some()
            || row.guest_path.is_some()
            || row.slot_required.is_some()
            || row.minimum_capacity_bytes.is_some()
            || row.release_contract_hash.is_some()
        {
            return Err(VolumeError::InvalidState(
                "historical lease has conflicting selection evidence",
            ));
        }
        return RunVolumeLease::historical_unproven(lease);
    }
    let origin = match row.selection_origin.as_str() {
        "explicit" => VolumeSelectionOrigin::Explicit,
        "legacy_declaration" => VolumeSelectionOrigin::LegacyDeclaration,
        "legacy_origin" => VolumeSelectionOrigin::LegacyOrigin,
        _ => {
            return Err(VolumeError::InvalidState(
                "unknown canonical lease provenance",
            ));
        }
    };
    let identity = RunVolumeIdentity::new(
        lease.run_id,
        AgentInstanceId::from_uuid(row.instance_id),
        AgentInstanceRevisionId::from_uuid(required(row.selected_revision_id)?),
        ReleaseId::from_uuid(required(row.selected_release_id)?),
        ReleaseAgentId::from_uuid(required(row.selected_release_agent_id)?),
        required(row.selected_project_id)?,
    )
    .map_err(metadata)?;
    let declaration = VolumeSlotDeclaration::new(
        CapabilitySlotKey::parse(required(row.slot_key)?).map_err(metadata)?,
        GuestMountPath::parse(required(row.guest_path)?).map_err(metadata)?,
        catalog::mode(&required(row.access_mode)?)?,
        required(row.slot_required)?,
        u64::try_from(required(row.minimum_capacity_bytes)?).map_err(metadata)?,
    )
    .map_err(metadata)?;
    let hash = AuthorityHash::from_bytes(
        required(row.release_contract_hash)?
            .try_into()
            .map_err(|_| VolumeError::InvalidState("invalid canonical lease contract hash"))?,
    );
    let selection = catalog::selection(identity, lease.volume_id, declaration, origin, hash)?;
    RunVolumeLease::selected(lease, selection)
}
