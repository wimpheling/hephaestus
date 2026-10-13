use crate::{errors::metadata, owned_provisioning::storage};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use volume_trait::{
    OwnedBackingPhase, OwnedPartialBirthPhase, OwnedPartialRetentionReceiptId,
    OwnedProvisioningClaim, PartialRetentionObservationHead, VolumeError,
};

#[derive(sqlx::FromRow)]
struct HeadRow {
    version: i64,
    backing_observation_id: Option<Uuid>,
    partial_receipt_id: Option<Uuid>,
    phase: String,
}
pub async fn exact_partial(
    tx: &mut Transaction<'_, Postgres>,
    claim: &OwnedProvisioningClaim,
) -> Result<(), VolumeError> {
    // Existing exact_active checks worker role, full original110receipt, full
    // purpose bytes, original111operation provenance and current volume CAS.
    // Its staged foundation additionally rejects permanently retired births.
    storage::exact_active(tx, claim).await?;
    let eligible: bool = sqlx::query_scalar("SELECT state='uninitialized' AND provisioning_state<>'ready' AND NOT EXISTS(SELECT 1 FROM volume_owned_backing_observations WHERE volume_id=$1 AND phase='ready') FROM agent_instance_state_volumes WHERE id=$1")
        .bind(claim.purpose.receipt().intent.registration().id().as_uuid()).fetch_one(&mut **tx).await.map_err(metadata)?;
    if eligible {
        Ok(())
    } else {
        Err(VolumeError::InvalidState(
            "Ready birth is not partial retention",
        ))
    }
}
pub async fn head(
    tx: &mut Transaction<'_, Postgres>,
    claim: &OwnedProvisioningClaim,
) -> Result<PartialRetentionObservationHead, VolumeError> {
    let row: Option<HeadRow> = sqlx::query_as("SELECT version,backing_observation_id,partial_receipt_id,phase FROM volume_owned_observation_heads WHERE volume_id=$1")
        .bind(claim.purpose.receipt().intent.registration().id().as_uuid()).fetch_optional(&mut **tx).await.map_err(metadata)?;
    let Some(row) = row else {
        // Unknown historic observations cannot be guessed into a current head.
        // Actual fresh custody-backed observation must establish the first head.
        return Ok(PartialRetentionObservationHead {
            version: 0,
            observation_id: None,
            backing_phase: None,
            partial_receipt_id: None,
        });
    };
    let version = u64::try_from(row.version).map_err(metadata)?;
    if version == 0 {
        return Err(VolumeError::IntentConflict);
    }
    match (row.backing_observation_id, row.partial_receipt_id) {
        (Some(id), None) if !id.is_nil() => {
            let phase: OwnedBackingPhase =
                serde_json::from_value(serde_json::Value::String(row.phase)).map_err(metadata)?;
            if phase == OwnedBackingPhase::Ready {
                return Err(VolumeError::InvalidState(
                    "Ready observation cannot be retired as partial",
                ));
            }
            Ok(PartialRetentionObservationHead {
                version,
                observation_id: Some(id),
                backing_phase: Some(phase),
                partial_receipt_id: None,
            })
        }
        (None, Some(id)) => {
            let _: OwnedPartialBirthPhase =
                serde_json::from_value(serde_json::Value::String(row.phase)).map_err(metadata)?;
            Ok(PartialRetentionObservationHead {
                version,
                observation_id: Some(id),
                backing_phase: None,
                partial_receipt_id: Some(
                    OwnedPartialRetentionReceiptId::from_uuid(id).map_err(metadata)?,
                ),
            })
        }
        _ => Err(VolumeError::IntentConflict),
    }
}
