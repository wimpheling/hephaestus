//! Authoritative exact-revision selection; caller execution authority is separate.

mod queries;

use capability_domain::{AuthorityHash, CapabilitySlotKey};
use runtime_types::{
    AgentInstanceId, AgentInstanceRevisionId, ReleaseAgentId, ReleaseId, RunId, VolumeId,
};
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use volume_domain::{
    RunVolumeIdentity, RunVolumeSelection, RunVolumeSelections, VolumeAccessMode, VolumeMountScope,
    VolumeSelectionOrigin, VolumeSlotBinding, VolumeSlotDeclaration, effective_volume_slots,
    validate_volume_bindings,
};
use volume_trait::VolumeError;

use super::lease_rows::lock_consumer_run;
use crate::errors::metadata;

#[derive(sqlx::FromRow)]
struct Execution {
    instance_id: Uuid,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    project_id: Uuid,
    state_volume_id: Option<Uuid>,
    volume_mode: String,
    requires_state: bool,
    runtime_contract: Value,
    runtime_contract_hash: Vec<u8>,
    admitted: bool,
    volume_selection_snapshot: Option<Value>,
}

#[derive(sqlx::FromRow)]
struct SelectedBinding {
    slot_key: String,
    volume_id: Uuid,
    access_mode: String,
    guest_path: String,
    slot_required: bool,
    minimum_capacity_bytes: i64,
    provenance: String,
    authorized: bool,
}

pub async fn check(
    tx: &mut Transaction<'_, Postgres>,
    expected: &RunVolumeSelections,
) -> Result<(), VolumeError> {
    if load(tx, expected.identity().run_id()).await? != *expected {
        return Err(VolumeError::InvalidState("exact run selections changed"));
    }
    Ok(())
}

pub async fn load(
    tx: &mut Transaction<'_, Postgres>,
    run_id: RunId,
) -> Result<RunVolumeSelections, VolumeError> {
    lock_consumer_run(tx, run_id).await?;
    load_locked(tx, run_id, false).await
}

// Qualified119 caller already holds early reservations and every canonical lock.
// The boolean is internal composition, not a caller-supplied admission label.
pub(super) async fn load_locked(
    tx: &mut Transaction<'_, Postgres>,
    run_id: RunId,
    invocation: bool,
) -> Result<RunVolumeSelections, VolumeError> {
    let row = if invocation {
        queries::invocation(tx, run_id).await?
    } else {
        queries::ordinary(tx, run_id).await?
    };
    if !row.admitted {
        return Err(VolumeError::InvalidState(
            "run is closed or unsupported for volume effects",
        ));
    }
    let identity = RunVolumeIdentity::new(
        run_id,
        AgentInstanceId::from_uuid(row.instance_id),
        AgentInstanceRevisionId::from_uuid(row.instance_revision_id),
        ReleaseId::from_uuid(row.release_id),
        ReleaseAgentId::from_uuid(row.release_agent_id),
        row.project_id,
    )
    .map_err(metadata)?;
    let authored: Vec<VolumeSlotDeclaration> = serde_json::from_value(
        row.runtime_contract
            .get("volume_slots")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .map_err(metadata)?;
    let declarations = effective_volume_slots(&authored, row.requires_state).map_err(metadata)?;
    let hash = AuthorityHash::from_bytes(
        row.runtime_contract_hash
            .as_slice()
            .try_into()
            .map_err(|_| VolumeError::InvalidState("invalid released contract hash"))?,
    );
    let selections = match row.volume_mode.as_str() {
        "legacy" => legacy(tx, &row, identity, &authored, &declarations, hash).await?,
        "named" => named(tx, identity, &declarations, hash).await?,
        _ => return Err(VolumeError::InvalidState("unknown volume binding mode")),
    };
    let set = RunVolumeSelections::new(identity, selections).map_err(metadata)?;
    if row
        .volume_selection_snapshot
        .as_ref()
        .is_some_and(|snapshot| serde_json::to_value(&set).as_ref().ok() != Some(snapshot))
    {
        return Err(VolumeError::InvalidState(
            "immutable run volume snapshot changed",
        ));
    }
    Ok(set)
}

async fn legacy(
    tx: &mut Transaction<'_, Postgres>,
    row: &Execution,
    identity: RunVolumeIdentity,
    authored: &[VolumeSlotDeclaration],
    declarations: &[VolumeSlotDeclaration],
    hash: AuthorityHash,
) -> Result<Vec<RunVolumeSelection>, VolumeError> {
    if !authored.is_empty() {
        return Err(VolumeError::InvalidState(
            "legacy mode cannot ignore authored slots",
        ));
    }
    if !row.requires_state {
        return Ok(Vec::new());
    }
    let id = row
        .state_volume_id
        .ok_or(VolumeError::InvalidState("legacy state pointer absent"))?;
    let proven: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_instance_state_volumes WHERE id=$1 AND instance_id=$2 AND project_id=$3 AND capacity_bytes>=1)")
        .bind(id).bind(row.instance_id).bind(row.project_id).fetch_one(&mut **tx).await.map_err(metadata)?;
    if !proven {
        return Err(VolumeError::InvalidState(
            "legacy volume origin is not proven",
        ));
    }
    let declaration = declarations
        .first()
        .ok_or(VolumeError::InvalidState("legacy state declaration absent"))?;
    Ok(vec![selection(
        identity,
        VolumeId::from_uuid(id),
        declaration.clone(),
        VolumeSelectionOrigin::LegacyOrigin,
        hash,
    )?])
}

async fn named(
    tx: &mut Transaction<'_, Postgres>,
    identity: RunVolumeIdentity,
    declarations: &[VolumeSlotDeclaration],
    hash: AuthorityHash,
) -> Result<Vec<RunVolumeSelection>, VolumeError> {
    let rows: Vec<SelectedBinding> = sqlx::query_as(
        "SELECT binding.slot_key,binding.volume_id,binding.access_mode,binding.guest_path,
         binding.slot_required,binding.minimum_capacity_bytes,binding.provenance,
         (binding.instance_id=$2 AND binding.release_agent_id=$3 AND binding.project_id=$4
          AND volume.project_id=$4 AND volume.capacity_bytes>=binding.minimum_capacity_bytes
          AND private_volume_mount_binding_is_declared(binding.instance_revision_id,binding.slot_key)
          AND EXISTS(SELECT 1 FROM agent_instance_volume_mount_grants grant_record
            WHERE grant_record.instance_revision_id=binding.instance_revision_id
            AND grant_record.instance_id=$2 AND grant_record.release_agent_id=$3
            AND grant_record.slot_key=binding.slot_key AND grant_record.volume_id=binding.volume_id
            AND grant_record.access_mode=binding.access_mode AND grant_record.release_contract_hash=$5
            AND private_volume_mount_source_is_live(grant_record.created_by,grant_record.volume_id)
            AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_revocations revoked WHERE revoked.grant_id=grant_record.id))
          AND check_permission('agent_instance',$2::text,'agent_attach','state_volume',binding.volume_id::text)=1) AS authorized
         FROM agent_instance_revision_volume_bindings binding
         JOIN agent_instance_state_volumes volume ON volume.id=binding.volume_id
         WHERE binding.instance_revision_id=$1 ORDER BY binding.slot_key",
    ).bind(identity.revision_id().as_uuid()).bind(identity.instance_id().as_uuid())
        .bind(identity.release_agent_id().as_uuid()).bind(identity.project_id()).bind(hash.as_bytes().as_slice())
        .fetch_all(&mut **tx).await.map_err(metadata)?;
    if rows.len() > volume_domain::MAX_VOLUME_SLOTS {
        return Err(VolumeError::InvalidState("too many stored volume bindings"));
    }
    let mut selections = Vec::with_capacity(rows.len());
    let mut bindings = Vec::with_capacity(rows.len());
    for row in rows {
        let key = CapabilitySlotKey::parse(row.slot_key).map_err(metadata)?;
        let declared = declarations
            .iter()
            .find(|slot| slot.slot() == &key)
            .ok_or(VolumeError::InvalidState("stored slot is undeclared"))?;
        if !row.authorized
            || row.access_mode != declared.access_mode().as_str()
            || row.guest_path != declared.guest_path().as_str()
            || row.slot_required != declared.required()
            || u64::try_from(row.minimum_capacity_bytes).ok()
                != Some(declared.minimum_capacity_bytes())
        {
            return Err(VolumeError::InvalidState(
                "stored binding or live mount grant differs",
            ));
        }
        let origin = match row.provenance.as_str() {
            "explicit" => VolumeSelectionOrigin::Explicit,
            "legacy_declaration" => VolumeSelectionOrigin::LegacyDeclaration,
            _ => {
                return Err(VolumeError::InvalidState(
                    "unsupported named binding provenance",
                ));
            }
        };
        bindings.push(VolumeSlotBinding::new(
            key,
            VolumeId::from_uuid(row.volume_id),
            declared.access_mode(),
        ));
        selections.push(selection(
            identity,
            VolumeId::from_uuid(row.volume_id),
            declared.clone(),
            origin,
            hash,
        )?);
    }
    validate_volume_bindings(declarations, &bindings).map_err(metadata)?;
    Ok(selections)
}

pub fn selection(
    identity: RunVolumeIdentity,
    volume_id: VolumeId,
    declaration: VolumeSlotDeclaration,
    origin: VolumeSelectionOrigin,
    hash: AuthorityHash,
) -> Result<RunVolumeSelection, VolumeError> {
    let scope = VolumeMountScope::new(
        identity.instance_id(),
        identity.revision_id(),
        identity.release_agent_id(),
        declaration.slot().clone(),
        volume_id,
        declaration.access_mode(),
        hash,
    )
    .map_err(metadata)?;
    RunVolumeSelection::new(identity, scope, declaration, origin).map_err(metadata)
}

pub fn mode(value: &str) -> Result<VolumeAccessMode, VolumeError> {
    match value {
        "read_only" => Ok(VolumeAccessMode::ReadOnly),
        "read_write" => Ok(VolumeAccessMode::ReadWrite),
        _ => Err(VolumeError::InvalidState("unknown exact access mode")),
    }
}
