use super::{PostgresReadyRetentionRepository, context};
use crate::{errors::metadata, owned_provisioning::storage};
use uuid::Uuid;
use volume_trait::{
    OwnedBackingObservation, OwnedReadyRetentionContext, OwnedReadyRetentionFact, VolumeError,
};

pub async fn record(
    repository: &PostgresReadyRetentionRepository,
    expected: &OwnedReadyRetentionContext,
    observation: &OwnedBackingObservation,
) -> Result<OwnedReadyRetentionFact, VolumeError> {
    expected.validate()?;
    let mut tx = repository.pool.begin().await.map_err(metadata)?;
    let actual = context::load(
        repository,
        &mut tx,
        &expected.original.claim,
        expected.closure.fields().id,
    )
    .await?;
    if actual != *expected {
        return Err(VolumeError::IntentConflict);
    }
    let prior = actual
        .original
        .observation
        .as_ref()
        .ok_or(VolumeError::IntentConflict)?;
    let mut compared = observation.fields().clone();
    compared.id = prior.fields().id;
    if compared != *prior.fields() {
        return Err(VolumeError::IntentConflict);
    }
    let json = serde_json::to_value(observation.fields()).map_err(metadata)?;
    let existing: Option<(Uuid, serde_json::Value, Uuid)> = sqlx::query_as(
        "SELECT closure_id,proof_json,event_id FROM volume_owned_ready_retention_facts WHERE id=$1",
    )
    .bind(observation.fields().id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(metadata)?;
    let event_id = if let Some((closure, proof, event)) = existing {
        if closure != expected.closure.fields().id || proof != json {
            return Err(VolumeError::IntentConflict);
        }
        event
    } else {
        storage::occurrence(&mut tx, observation.fields().id).await?;
        let event: Uuid = sqlx::query_scalar(
            "SELECT event_id FROM append_application_event($1,'project',$2,'project',$2,'project.changed','updated',NULL,$3,$1)")
            .bind(observation.fields().id).bind(expected.closure.fields().project_id)
            .bind(expected.closure.fields().volume_id.as_uuid()).fetch_one(&mut *tx).await.map_err(metadata)?;
        sqlx::query("INSERT INTO volume_owned_ready_retention_facts(id,closure_id,proof_json,evidence_hash,event_id)
            VALUES($1,$2,$3,sha256(convert_to($3::jsonb::text,'UTF8')),$4)")
            .bind(observation.fields().id).bind(expected.closure.fields().id).bind(json).bind(event)
            .execute(&mut *tx).await.map_err(metadata)?;
        event
    };
    tx.commit().await.map_err(metadata)?;
    Ok(OwnedReadyRetentionFact {
        id: observation.fields().id,
        closure_id: expected.closure.fields().id,
        observation: observation.clone(),
        event_id,
    })
}
