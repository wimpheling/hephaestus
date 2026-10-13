//! Exact worker-only first-completion fence; no filesystem proof or mutation.

use volume_trait::{OwnedProvisioningClaim, VolumeError};

use super::storage;
use crate::{PostgresVolumeMetadataRepository, errors::metadata};

pub async fn assert_unfinished(
    repository: &PostgresVolumeMetadataRepository,
    claim: &OwnedProvisioningClaim,
) -> Result<(), VolumeError> {
    let mut tx = repository.pool.begin().await.map_err(metadata)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(metadata)?;
    storage::role(&mut tx, "hephaestus_worker").await?;
    storage::original_receipt(&mut tx, &claim.purpose).await?;
    if !storage::check_purpose(&mut tx, &claim.purpose).await? {
        return Err(VolumeError::IntentConflict);
    }
    let volume = claim.purpose.receipt().intent.registration().id().as_uuid();
    storage::require_live(&mut tx, volume).await?;
    let operation = storage::operation(&mut tx, claim.operation_id)
        .await?
        .ok_or(VolumeError::IntentConflict)?;
    if operation.claim(&claim.purpose)? != *claim {
        return Err(VolumeError::IntentConflict);
    }
    let generation: i64 = sqlx::query_scalar(
        "SELECT provisioning_generation FROM agent_instance_state_volumes WHERE id=$1",
    )
    .bind(volume)
    .fetch_one(&mut *tx)
    .await
    .map_err(metadata)?;
    if u64::try_from(generation).map_err(metadata)? != claim.generation {
        return Err(VolumeError::StaleLease);
    }
    sqlx::query("SELECT assert_recipe_owned_first_ready($1,$2,$3,$4)")
        .bind(claim.purpose.receipt().intent.registration().id().as_uuid())
        .bind(claim.operation_id)
        .bind(i64::try_from(claim.generation).map_err(metadata)?)
        .bind(claim.purpose.hash().as_slice())
        .execute(&mut *tx)
        .await
        .map_err(metadata)?;
    tx.commit().await.map_err(metadata)
}
