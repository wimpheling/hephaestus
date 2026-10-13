use super::mapping;
use crate::{effect_context::Context, execution_transitions, hydration, repository_error};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{DeploymentError, EffectClaim, PreparedOwnedVolumeCreation};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use volume_domain::{VolumeCreationIdentity, VolumeCreationIdentityWire};

#[derive(sqlx::FromRow)]
struct Row {
    mapping_version: i32,
    claim_digest: Vec<u8>,
    creation_json: serde_json::Value,
    volume_id: Uuid,
    project_id: Uuid,
    capacity_bytes: i64,
    filesystem_uuid: Uuid,
}

pub async fn load(
    tx: &mut Transaction<'_, Postgres>,
    claim: &EffectClaim,
) -> Result<Option<PreparedOwnedVolumeCreation>, DeploymentError> {
    let row: Option<Row> = sqlx::query_as("SELECT mapping_version,claim_digest,creation_json,volume_id,project_id,capacity_bytes,filesystem_uuid FROM recipe_owned_volume_preparations WHERE attempt_id=$1")
        .bind(claim.provenance.attempt_id.as_uuid()).fetch_optional(&mut **tx).await.map_err(repository_error)?;
    row.map(|row| {
        if row.mapping_version != 1 {
            return Err(DeploymentError::IntentMismatch);
        }
        let wire: VolumeCreationIdentityWire = serde_json::from_value(row.creation_json)
            .map_err(|_| DeploymentError::IntentMismatch)?;
        Ok(PreparedOwnedVolumeCreation {
            mapping_version: 1,
            claim_digest: hydration::digest(&row.claim_digest)?,
            creation: VolumeCreationIdentity::try_from(wire)
                .map_err(|_| DeploymentError::IntentMismatch)?,
            volume_id: runtime_types::VolumeId::from_uuid(row.volume_id),
            project_id: row.project_id,
            capacity_bytes: hydration::unsigned(row.capacity_bytes)?,
            filesystem_uuid: row.filesystem_uuid,
        })
    })
    .transpose()
}

pub async fn append(
    context: &mut Context<'_>,
    identity: &AuthenticatedIdentity,
    claim: &EffectClaim,
    preparation: &PreparedOwnedVolumeCreation,
) -> Result<(), DeploymentError> {
    let occurrence = Uuid::new_v5(
        &claim.provenance.attempt_id.as_uuid(),
        b"hephaestus-recipe-owned-volume-preparation-v1\0",
    );
    let (event, cursor, version) = execution_transitions::event(
        &mut context.tx,
        identity,
        occurrence,
        context.organization,
        context.snapshot.intent.project_id(),
    )
    .await?;
    sqlx::query("INSERT INTO recipe_owned_volume_preparations(attempt_id,deployment_id,project_id,resource_name,mapping_version,canonical_claim,claim_digest,scope_id,effect_id,registration_operation_id,creation_json,volume_id,capacity_bytes,filesystem_uuid,actor_id,request_id,event_id,event_cursor,event_aggregate_version) VALUES($1,$2,$3,$4,1,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)")
        .bind(claim.provenance.attempt_id.as_uuid()).bind(claim.deployment_id.as_uuid()).bind(preparation.project_id).bind(claim.resource.as_str())
        .bind(mapping::canonical_claim(claim)?).bind(preparation.claim_digest.as_bytes().as_slice())
        .bind(preparation.creation.fields().scope_id.as_uuid()).bind(preparation.creation.fields().effect_id).bind(preparation.creation.fields().operation_id.as_uuid())
        .bind(serde_json::to_value(preparation.creation.fields()).map_err(|_|DeploymentError::Serialization)?)
        .bind(preparation.volume_id.as_uuid()).bind(i64::try_from(preparation.capacity_bytes).map_err(|_|DeploymentError::IntentMismatch)?)
        .bind(preparation.filesystem_uuid).bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid()).bind(event).bind(cursor).bind(version)
        .execute(&mut *context.tx).await.map_err(repository_error)?;
    Ok(())
}
