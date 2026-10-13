use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use volume_trait::{
    OwnedBackingObservation, OwnedBackingObservationWire, OwnedBackingPurpose,
    OwnedProvisioningClaim, OwnedProvisioningContext, VolumeError,
};

use crate::errors::metadata;

#[derive(sqlx::FromRow)]
pub struct OperationRow {
    pub id: Uuid,
    pub volume_id: Uuid,
    pub purpose_hash: Vec<u8>,
    pub expected_generation: i64,
    pub generation: i64,
    pub actor_id: Uuid,
    pub request_id: Uuid,
    pub event_id: Uuid,
}
impl OperationRow {
    pub fn claim(
        &self,
        purpose: &OwnedBackingPurpose,
    ) -> Result<OwnedProvisioningClaim, VolumeError> {
        if self.volume_id != purpose.receipt().intent.registration().id().as_uuid()
            || self.purpose_hash.as_slice() != purpose.hash()
            || self.actor_id != purpose.receipt().intent.creation().fields().actor_id
        {
            return Err(VolumeError::IntentConflict);
        }
        Ok(OwnedProvisioningClaim {
            purpose: purpose.clone(),
            operation_id: self.id,
            generation: u64::try_from(self.generation).map_err(metadata)?,
            request_id: self.request_id,
            event_id: self.event_id,
        })
    }
}
pub async fn role(tx: &mut Transaction<'_, Postgres>, expected: &str) -> Result<(), VolumeError> {
    let role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&mut **tx)
        .await
        .map_err(metadata)?;
    if role == expected {
        Ok(())
    } else {
        Err(VolumeError::PermissionDenied)
    }
}
pub async fn operation(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<Option<OperationRow>, VolumeError> {
    sqlx::query_as("SELECT * FROM volume_owned_provisioning_operations WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(metadata)
}
pub async fn check_purpose(
    tx: &mut Transaction<'_, Postgres>,
    purpose: &OwnedBackingPurpose,
) -> Result<bool, VolumeError> {
    let bytes: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT canonical_bytes FROM volume_owned_backing_purposes WHERE volume_id=$1",
    )
    .bind(purpose.receipt().intent.registration().id().as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(metadata)?;
    match bytes {
        Some(bytes) if bytes == purpose.canonical_bytes() => Ok(true),
        Some(_) => Err(VolumeError::IntentConflict),
        None => Ok(false),
    }
}
pub async fn original_receipt(
    tx: &mut Transaction<'_, Postgres>,
    purpose: &OwnedBackingPurpose,
) -> Result<(), VolumeError> {
    let receipt = purpose.receipt();
    let exact: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM volume_creation_seals seal JOIN agent_instance_state_volumes volume ON volume.id=seal.volume_id WHERE seal.id=$1 AND seal.volume_id=$2 AND seal.canonical_bytes=$3 AND seal.registration_hash=$4 AND seal.first_request_id=$5 AND seal.event_id=$6 AND seal.event_cursor=$7 AND seal.event_aggregate_version=$8 AND volume.creation_seal_id=seal.id AND volume.project_id=seal.project_id AND volume.filesystem_uuid=seal.filesystem_uuid AND volume.capacity_bytes=seal.capacity_bytes)")
        .bind(receipt.seal_id.as_uuid()).bind(receipt.intent.registration().id().as_uuid())
        .bind(receipt.intent.canonical_bytes()).bind(receipt.registration_hash.as_slice())
        .bind(receipt.first_request_id).bind(receipt.event_id).bind(receipt.event_cursor)
        .bind(receipt.event_aggregate_version).fetch_one(&mut **tx).await.map_err(metadata)?;
    if exact {
        Ok(())
    } else {
        Err(VolumeError::IntentConflict)
    }
}
pub async fn context(
    tx: &mut Transaction<'_, Postgres>,
    claim: OwnedProvisioningClaim,
) -> Result<OwnedProvisioningContext, VolumeError> {
    let proof: Option<serde_json::Value> = sqlx::query_scalar("SELECT observation.proof_json FROM volume_owned_backing_observations observation JOIN application_events event ON event.id=observation.event_id WHERE observation.operation_id=$1 ORDER BY event.cursor DESC LIMIT 1")
        .bind(claim.operation_id).fetch_optional(&mut **tx).await.map_err(metadata)?;
    let observation = proof
        .map(|proof| {
            serde_json::from_value::<OwnedBackingObservationWire>(proof)
                .map_err(metadata)
                .and_then(OwnedBackingObservation::try_from)
        })
        .transpose()?;
    Ok(OwnedProvisioningContext { claim, observation })
}
pub async fn exact_active(
    tx: &mut Transaction<'_, Postgres>,
    claim: &OwnedProvisioningClaim,
) -> Result<(), VolumeError> {
    role(tx, "hephaestus_worker").await?;
    original_receipt(tx, &claim.purpose).await?;
    if !check_purpose(tx, &claim.purpose).await? {
        return Err(VolumeError::IntentConflict);
    }
    let generation: i64 = sqlx::query_scalar(
        "SELECT provisioning_generation FROM agent_instance_state_volumes WHERE id=$1 FOR UPDATE",
    )
    .bind(claim.purpose.receipt().intent.registration().id().as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(metadata)?;
    require_live(
        tx,
        claim.purpose.receipt().intent.registration().id().as_uuid(),
    )
    .await?;
    let row = operation(tx, claim.operation_id)
        .await?
        .ok_or(VolumeError::IntentConflict)?;
    if row.claim(&claim.purpose)? != *claim {
        return Err(VolumeError::IntentConflict);
    }
    if u64::try_from(generation).map_err(metadata)? != claim.generation {
        return Err(VolumeError::StaleLease);
    }
    Ok(())
}
pub async fn require_live(
    tx: &mut Transaction<'_, Postgres>,
    volume_id: Uuid,
) -> Result<(), VolumeError> {
    let live: bool = sqlx::query_scalar("SELECT owned_birth_is_live($1)")
        .bind(volume_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(metadata)?;
    if live {
        Ok(())
    } else {
        Err(VolumeError::InvalidState(
            "owned birth is permanently retired",
        ))
    }
}
pub async fn occurrence(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<(), VolumeError> {
    sqlx::query("SELECT set_config('hephaestus.occurrence_id',$1,true)")
        .bind(id.to_string())
        .execute(&mut **tx)
        .await
        .map_err(metadata)?;
    Ok(())
}
