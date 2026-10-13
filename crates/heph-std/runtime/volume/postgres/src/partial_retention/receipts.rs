use super::storage;
use crate::{
    PostgresVolumeMetadataRepository, errors::metadata, owned_provisioning::storage as provisioning,
};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use volume_trait::{
    OwnedPartialBirthObservation, OwnedPartialBirthObservationWire, OwnedPartialRetentionContext,
    OwnedPartialRetentionReceipt, OwnedPartialRetentionReceiptId, VolumeError,
};

#[derive(sqlx::FromRow)]
struct ReceiptRow {
    operation_id: Uuid,
    proof_json: serde_json::Value,
    event_id: Uuid,
}
pub async fn record(
    repository: &PostgresVolumeMetadataRepository,
    context: &OwnedPartialRetentionContext,
    observation: &OwnedPartialBirthObservation,
) -> Result<OwnedPartialRetentionReceipt, VolumeError> {
    let mut tx = repository.pool.begin().await.map_err(metadata)?;
    storage::exact_partial(&mut tx, &context.claim).await?;
    check_input(context, observation)?;
    let wire = serde_json::to_value(observation.fields()).map_err(metadata)?;
    let existing: Option<ReceiptRow> = sqlx::query_as(
        "SELECT operation_id,proof_json,event_id FROM volume_owned_partial_receipts WHERE id=$1",
    )
    .bind(observation.fields().id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(metadata)?;
    let current_head = storage::head(&mut tx, &context.claim).await?;
    let event_id = if let Some(row) = existing {
        if row.operation_id != context.claim.operation_id
            || row.proof_json != wire
            || current_head.observation_id != Some(observation.fields().id)
            || current_head.version != observation.fields().observation_version
            || current_head
                .partial_receipt_id
                .map(OwnedPartialRetentionReceiptId::as_uuid)
                != Some(observation.fields().id)
        {
            return Err(VolumeError::IntentConflict);
        }
        // Decode/revalidate database JSON; a persisted row is not a validated DTO.
        let parsed = OwnedPartialBirthObservation::try_from(
            serde_json::from_value::<OwnedPartialBirthObservationWire>(row.proof_json)
                .map_err(metadata)?,
        )
        .map_err(metadata)?;
        if parsed != *observation {
            return Err(VolumeError::IntentConflict);
        }
        row.event_id
    } else {
        if current_head != context.head {
            return Err(VolumeError::StaleLease);
        }
        append(&mut tx, context, observation, wire).await?
    };
    tx.commit().await.map_err(metadata)?;
    Ok(OwnedPartialRetentionReceipt {
        id: OwnedPartialRetentionReceiptId::from_uuid(observation.fields().id).map_err(metadata)?,
        claim: context.claim.clone(),
        observation: observation.clone(),
        event_id,
    })
}
fn check_input(
    context: &OwnedPartialRetentionContext,
    observation: &OwnedPartialBirthObservation,
) -> Result<(), VolumeError> {
    let fields = observation.fields();
    let purpose = &context.claim.purpose;
    let receipt = purpose.receipt();
    let resource = receipt.intent.registration();
    if fields.volume_id != resource.id().as_uuid()
        || fields.project_id != resource.project_id()
        || fields.seal_id != receipt.seal_id
        || !observation.matches_creation(receipt.intent.creation())
        || fields.provisioning_operation_id != context.claim.operation_id
        || fields.generation != context.claim.generation
        || fields.purpose_hash != purpose.hash()
        || fields.owner_namespace != purpose.owner_namespace()
        || fields.declared_capacity_bytes != resource.capacity_bytes()
        || context.head.version.checked_add(1) != Some(fields.observation_version)
    {
        return Err(VolumeError::IntentConflict);
    }
    Ok(())
}
async fn append(
    tx: &mut Transaction<'_, Postgres>,
    context: &OwnedPartialRetentionContext,
    observation: &OwnedPartialBirthObservation,
    wire: serde_json::Value,
) -> Result<Uuid, VolumeError> {
    let fields = observation.fields();
    provisioning::occurrence(tx, fields.id).await?;
    // Explicit committed-outbox event; no canonical volume state/generation is
    // mutated just to manufacture a worker receipt or a physical Ready claim.
    let event_id: Uuid = sqlx::query_scalar("SELECT event_id FROM append_application_event($1,'project',$2,'project',$2,'project.changed','updated',NULL,$3,$1)")
        .bind(fields.id).bind(fields.project_id).bind(fields.volume_id).fetch_one(&mut **tx).await.map_err(metadata)?;
    sqlx::query("INSERT INTO volume_owned_partial_receipts(id,volume_id,project_id,seal_id,operation_id,generation,purpose_hash,previous_observation_id,previous_observation_version,observation_version,phase,declared_capacity_bytes,actual_length_bytes,proof_json,evidence_hash,event_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,sha256(convert_to($14::jsonb::text,'UTF8')),$15)")
        .bind(fields.id).bind(fields.volume_id).bind(fields.project_id).bind(fields.seal_id.as_uuid())
        .bind(context.claim.operation_id).bind(i64::try_from(fields.generation).map_err(metadata)?)
        .bind(fields.purpose_hash.as_slice()).bind(context.head.observation_id)
        .bind(i64::try_from(context.head.version).map_err(metadata)?).bind(i64::try_from(fields.observation_version).map_err(metadata)?)
        .bind(fields.phase.as_str()).bind(i64::try_from(fields.declared_capacity_bytes).map_err(metadata)?)
        .bind(i64::try_from(fields.actual_length_bytes).map_err(metadata)?).bind(wire).bind(event_id)
        .execute(&mut **tx).await.map_err(metadata)?;
    Ok(event_id)
}
