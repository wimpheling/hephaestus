//! Canonical typed-import validation and exact replay evidence.

use capability_domain::AuthorityHash;
use release_domain::{ParameterDeclaration, ParameterDocument};
use release_service::ImportAgentWithVolumes;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use volume_domain::{
    VolumeSlotBinding, VolumeSlotDeclaration, effective_volume_slots, validate_volume_bindings,
};

use crate::{AuthenticatedIdentity, ReleaseServiceError, RuntimePolicy, policy_from_contract};

#[derive(sqlx::FromRow)]
pub struct PublishedVolumeAgent {
    pub family_id: Uuid,
    pub parameter_schema: Value,
    pub secret_slot_schema: Value,
    pub runtime_contract: Value,
    pub runtime_contract_hash: Vec<u8>,
    pub requires_state: bool,
    pub publication_mode: String,
}

pub struct ImportInputs {
    pub parameters: ParameterDocument,
    pub effective_policy: RuntimePolicy,
    pub declarations: Vec<VolumeSlotDeclaration>,
    pub contract_hash: AuthorityHash,
    pub input_hash: [u8; 32],
}

pub async fn resolve_inputs(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    command: &mut ImportAgentWithVolumes,
    release: &PublishedVolumeAgent,
) -> Result<ImportInputs, ReleaseServiceError> {
    if [
        command.instance_id.as_uuid(),
        command.revision_id.as_uuid(),
        command.release_id.as_uuid(),
        command.release_agent_id.as_uuid(),
        command.project_id.as_uuid(),
    ]
    .iter()
    .any(Uuid::is_nil)
        || command.platform_policy_version.is_empty()
        || command.platform_policy_version.len() > 128
    {
        return Err(ReleaseServiceError::InvalidVolumeSelection);
    }
    let slots: Vec<VolumeSlotDeclaration> = serde_json::from_value(
        release
            .runtime_contract
            .get("volume_slots")
            .cloned()
            .unwrap_or_else(|| json!([])),
    )?;
    let declarations = effective_volume_slots(&slots, release.requires_state)
        .map_err(|_| ReleaseServiceError::InvalidVolumeSelection)?;
    command
        .volumes
        .sort_by(|left, right| left.slot.cmp(&right.slot));
    let bindings = command
        .volumes
        .iter()
        .map(|selection| {
            VolumeSlotBinding::new(
                selection.slot.clone(),
                selection.volume_id,
                selection.access_mode,
            )
        })
        .collect::<Vec<_>>();
    validate_volume_bindings(&declarations, &bindings)
        .map_err(|_| ReleaseServiceError::InvalidVolumeSelection)?;
    validate_remaining_requirements(tx, command, release, &declarations).await?;
    validate_resources(tx, command, &declarations).await?;
    let parameters = ParameterDocument::resolve(
        &serde_json::from_value::<Vec<ParameterDeclaration>>(release.parameter_schema.clone())?,
        &command.parameters,
    )
    .map_err(ReleaseServiceError::InvalidParameters)?;
    let effective_policy = RuntimePolicy::resolve(
        &policy_from_contract(&release.runtime_contract)?,
        &command.selected_policy,
        &command.platform_policy,
    )?;
    let contract_hash = AuthorityHash::from_bytes(
        release
            .runtime_contract_hash
            .clone()
            .try_into()
            .map_err(|_| ReleaseServiceError::InvalidStoredData)?,
    );
    let input_hash = import_hash(identity, command, &parameters)?;
    Ok(ImportInputs {
        parameters,
        effective_policy,
        declarations,
        contract_hash,
        input_hash,
    })
}

async fn validate_remaining_requirements(
    tx: &mut Transaction<'_, Postgres>,
    command: &ImportAgentWithVolumes,
    release: &PublishedVolumeAgent,
    declarations: &[VolumeSlotDeclaration],
) -> Result<(), ReleaseServiceError> {
    let generic: Vec<(String, bool)> = sqlx::query_as("SELECT slot_key,slot_required FROM release_capability_requirements WHERE release_agent_id=$1")
        .bind(command.release_agent_id.as_uuid()).fetch_all(&mut **tx).await?;
    if release.publication_mode != "proposal"
        || generic.iter().any(|(key, required)| {
            *required
                || declarations
                    .iter()
                    .any(|declaration| declaration.slot().as_str() == key)
        })
        || release
            .secret_slot_schema
            .as_array()
            .ok_or(ReleaseServiceError::InvalidStoredData)?
            .iter()
            .any(|slot| slot.get("required").and_then(Value::as_bool) == Some(true))
    {
        return Err(ReleaseServiceError::CapabilityResourceUnavailable);
    }
    Ok(())
}

async fn validate_resources(
    tx: &mut Transaction<'_, Postgres>,
    command: &ImportAgentWithVolumes,
    declarations: &[VolumeSlotDeclaration],
) -> Result<(), ReleaseServiceError> {
    let mut grant_ids = std::collections::BTreeSet::new();
    for selection in &command.volumes {
        let declaration = declarations
            .iter()
            .find(|declaration| declaration.slot() == &selection.slot)
            .ok_or(ReleaseServiceError::InvalidVolumeSelection)?;
        if selection.guest_path != *declaration.guest_path()
            || !grant_ids.insert(selection.grant_id.as_uuid())
            || selection.volume_id.as_uuid().is_nil()
        {
            return Err(ReleaseServiceError::InvalidVolumeSelection);
        }
        let capacity: Option<i64> = sqlx::query_scalar(
            "SELECT capacity_bytes FROM agent_instance_state_volumes WHERE id=$1 AND project_id=$2",
        )
        .bind(selection.volume_id.as_uuid())
        .bind(command.project_id.as_uuid())
        .fetch_optional(&mut **tx)
        .await?;
        if capacity
            .and_then(|bytes| u64::try_from(bytes).ok())
            .is_none_or(|bytes| bytes < declaration.minimum_capacity_bytes())
        {
            return Err(ReleaseServiceError::InvalidVolumeSelection);
        }
    }
    Ok(())
}

fn import_hash(
    identity: &AuthenticatedIdentity,
    command: &ImportAgentWithVolumes,
    parameters: &ParameterDocument,
) -> Result<[u8; 32], ReleaseServiceError> {
    let selections = command.volumes.iter().map(|slot| json!({"slot":slot.slot,"guest_path":slot.guest_path,"access_mode":slot.access_mode,"volume_id":slot.volume_id,"grant_id":slot.grant_id})).collect::<Vec<_>>();
    Ok(Sha256::digest(serde_json::to_vec(&json!({
        "version":1,"operation":"import_agent_with_volumes","actor":identity.user_id,
        "instance":command.instance_id,"revision":command.revision_id,"release":command.release_id,"agent":command.release_agent_id,"project":command.project_id,"name":command.name.as_str(),"parameters":parameters.values(),"selected_policy":command.selected_policy,"platform_policy":command.platform_policy,"platform_policy_version":command.platform_policy_version,"volumes":selections
    }))?).into())
}

#[derive(sqlx::FromRow)]
struct ExactCommandRow {
    operation: String,
    aggregate_id: Uuid,
    secondary_id: Option<Uuid>,
    input_hash: Option<Vec<u8>>,
}

pub async fn exact_replay(
    tx: &mut Transaction<'_, Postgres>,
    key: release_domain::ReleaseCommandKey,
    operation: &str,
    input_hash: &[u8; 32],
) -> Result<Option<(Uuid, Option<Uuid>)>, ReleaseServiceError> {
    let lock_key = i64::from_be_bytes(
        *key.as_bytes()
            .first_chunk::<8>()
            .ok_or(ReleaseServiceError::InvalidStoredData)?,
    );
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock_key)
        .execute(&mut **tx)
        .await?;
    let row: Option<ExactCommandRow> = sqlx::query_as("SELECT operation,aggregate_id,secondary_id,input_hash FROM release_command_inbox WHERE command_key=$1")
        .bind(key.as_bytes().as_slice()).fetch_optional(&mut **tx).await?;
    match row {
        Some(row)
            if row.operation == operation
                && row.input_hash.as_deref() == Some(input_hash.as_slice()) =>
        {
            Ok(Some((row.aggregate_id, row.secondary_id)))
        }
        Some(_) => Err(ReleaseServiceError::IdempotencyConflict),
        None => Ok(None),
    }
}

pub async fn record_exact(
    tx: &mut Transaction<'_, Postgres>,
    key: release_domain::ReleaseCommandKey,
    operation: &str,
    aggregate: Uuid,
    secondary: Option<Uuid>,
    identity: &AuthenticatedIdentity,
    hash: &[u8; 32],
) -> Result<(), ReleaseServiceError> {
    sqlx::query("INSERT INTO release_command_inbox(command_key,operation,aggregate_id,secondary_id,actor_id,request_id,input_hash) VALUES($1,$2,$3,$4,$5,$6,$7)")
        .bind(key.as_bytes().as_slice()).bind(operation).bind(aggregate).bind(secondary).bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid()).bind(hash.as_slice()).execute(&mut **tx).await?;
    Ok(())
}
