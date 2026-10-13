use sqlx::{Postgres, Transaction};
use volume_trait::{
    OwnedBackingObservation, OwnedProvisioningClaim, OwnedProvisioningContext, VolumeError,
};

use super::storage;
use crate::{PostgresVolumeMetadataRepository, errors::metadata};

pub async fn inspect(
    repository: &PostgresVolumeMetadataRepository,
    claim: &OwnedProvisioningClaim,
) -> Result<OwnedProvisioningContext, VolumeError> {
    let mut tx = repository.pool.begin().await.map_err(metadata)?;
    storage::exact_active(&mut tx, claim).await?;
    let result = storage::context(&mut tx, claim.clone()).await?;
    tx.commit().await.map_err(metadata)?;
    Ok(result)
}
pub async fn record(
    repository: &PostgresVolumeMetadataRepository,
    claim: &OwnedProvisioningClaim,
    observation: &OwnedBackingObservation,
) -> Result<OwnedProvisioningContext, VolumeError> {
    let mut tx = repository.pool.begin().await.map_err(metadata)?;
    storage::exact_active(&mut tx, claim).await?;
    let fields = observation.fields();
    if fields.generation != claim.generation
        || fields.purpose_hash != claim.purpose.hash()
        || fields.capacity_bytes
            != claim
                .purpose
                .receipt()
                .intent
                .registration()
                .capacity_bytes()
        || fields.filesystem.as_ref().is_some_and(|filesystem| {
            filesystem.filesystem_uuid
                != claim
                    .purpose
                    .receipt()
                    .intent
                    .registration()
                    .filesystem_uuid()
        })
    {
        return Err(VolumeError::IntentConflict);
    }
    let wire = serde_json::to_value(fields).map_err(metadata)?;
    let recorded: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT proof_json FROM volume_owned_backing_observations WHERE id=$1")
            .bind(fields.id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(metadata)?;
    if let Some(recorded) = recorded {
        if recorded != wire {
            return Err(VolumeError::IntentConflict);
        }
    } else {
        append(&mut tx, claim, observation, wire).await?;
    }
    let result = storage::context(&mut tx, claim.clone()).await?;
    tx.commit().await.map_err(metadata)?;
    Ok(result)
}
async fn append(
    tx: &mut Transaction<'_, Postgres>,
    claim: &OwnedProvisioningClaim,
    observation: &OwnedBackingObservation,
    wire: serde_json::Value,
) -> Result<(), VolumeError> {
    let fields = observation.fields();
    let resource = claim.purpose.receipt().intent.registration();
    let generation = i64::try_from(claim.generation).map_err(metadata)?;
    storage::occurrence(tx, fields.id).await?;
    sqlx::query("INSERT INTO volume_owned_transition_proofs(id,volume_id,operation_id,generation,previous_generation,mode) VALUES($1,$2,$3,$4,$4,$5)")
        .bind(fields.id).bind(resource.id().as_uuid()).bind(claim.operation_id).bind(generation)
        .bind(fields.phase.as_str()).execute(&mut **tx).await.map_err(metadata)?;
    sqlx::query("UPDATE agent_instance_state_volumes SET provisioning_state=CASE $3 WHEN 'recorded' THEN 'creating' WHEN 'format_intent' THEN 'formatting' WHEN 'ready' THEN 'ready' ELSE 'uncertain' END,state=CASE WHEN $3='ready' THEN 'ready' ELSE 'uninitialized' END,updated_at=now() WHERE id=$1 AND provisioning_generation=$2")
        .bind(resource.id().as_uuid()).bind(generation).bind(fields.phase.as_str()).execute(&mut **tx).await.map_err(metadata)?;
    sqlx::query("INSERT INTO volume_owned_backing_observations(id,operation_id,volume_id,generation,birth_observation_id,phase,proof_json,evidence_hash,event_id) SELECT $1,$2,$3,$4,COALESCE((SELECT id FROM volume_owned_backing_observations WHERE volume_id=$3 AND birth_observation_id=id LIMIT 1),$1),$5,$6,sha256(convert_to($6::jsonb::text,'UTF8')),event.id FROM application_events event WHERE event.occurrence_id=$1 AND event.scope_kind='project' AND event.scope_id=$7 AND event.aggregate_type='project' AND event.aggregate_id=$7 AND event.event_type='project.changed'")
        .bind(fields.id).bind(claim.operation_id).bind(resource.id().as_uuid()).bind(generation)
        .bind(fields.phase.as_str()).bind(wire).bind(resource.project_id()).execute(&mut **tx).await.map_err(metadata)?;
    Ok(())
}
