use super::PostgresReadyRetentionRepository;
use crate::{errors::metadata, owned_provisioning::storage};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use volume_trait::{
    OwnedProvisioningClaim, OwnedReadyRetentionClosure, OwnedReadyRetentionClosureWire,
    OwnedReadyRetentionContext, VolumeError,
};

pub async fn load(
    repository: &PostgresReadyRetentionRepository,
    tx: &mut Transaction<'_, Postgres>,
    claim: &OwnedProvisioningClaim,
    closure_id: Uuid,
) -> Result<OwnedReadyRetentionContext, VolumeError> {
    storage::role(tx, "hephaestus_worker").await?;
    storage::original_receipt(tx, &claim.purpose).await?;
    if !storage::check_purpose(tx, &claim.purpose).await? {
        return Err(VolumeError::IntentConflict);
    }
    let operation = storage::operation(tx, claim.operation_id)
        .await?
        .ok_or(VolumeError::IntentConflict)?;
    if operation.claim(&claim.purpose)? != *claim {
        return Err(VolumeError::IntentConflict);
    }
    sqlx::query("SELECT assert_owned_ready_retention($1,$2,$3,$4,$5,$6)")
        .bind(closure_id)
        .bind(claim.operation_id)
        .bind(i64::try_from(claim.generation).map_err(metadata)?)
        .bind(claim.purpose.hash().as_slice())
        .bind(&repository.namespace)
        .bind(&repository.host)
        .execute(&mut **tx)
        .await
        .map_err(metadata)?;
    hydrate(tx, claim, closure_id).await
}

pub(super) async fn hydrate(
    tx: &mut Transaction<'_, Postgres>,
    claim: &OwnedProvisioningClaim,
    closure_id: Uuid,
) -> Result<OwnedReadyRetentionContext, VolumeError> {
    let json: serde_json::Value = sqlx::query_scalar(
        "SELECT jsonb_build_object('id',id,'volume_id',volume_id,'project_id',project_id,
         'creation_operation_id',creation_operation_id,'provisioning_operation_id',provisioning_operation_id,
         'generation',generation,'purpose_hash',to_jsonb(ARRAY(SELECT get_byte(purpose_hash,idx) FROM generate_series(0,31) idx)),
         'ready_observation_id',ready_observation_id,'vm_provider_namespace',vm_provider_namespace,'vm_host_id',vm_host_id)
         FROM volume_owned_ready_retention_closures WHERE id=$1")
        .bind(closure_id).fetch_one(&mut **tx).await.map_err(metadata)?;
    let closure = OwnedReadyRetentionClosure::try_from(
        serde_json::from_value::<OwnedReadyRetentionClosureWire>(json).map_err(metadata)?,
    )?;
    let original = storage::context(tx, claim.clone()).await?;
    let result = OwnedReadyRetentionContext { closure, original };
    result.validate()?;
    Ok(result)
}
