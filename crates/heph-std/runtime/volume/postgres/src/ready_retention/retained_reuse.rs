use super::{PostgresReadyRetentionRepository, context};
use crate::{errors::metadata, owned_provisioning::storage};
use async_trait::async_trait;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use volume_trait::{
    OwnedProvisioningClaim, OwnedRetainedReuseContext, VolumeError, VolumeRetainedReuseRepository,
};

#[async_trait]
impl VolumeRetainedReuseRepository for PostgresReadyRetentionRepository {
    async fn retained_reuse_context(
        &self,
        claim: &OwnedProvisioningClaim,
        closure_id: Uuid,
        retention_operation_id: Uuid,
    ) -> Result<OwnedRetainedReuseContext, VolumeError> {
        let mut tx = self.pool.begin().await.map_err(metadata)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(metadata)?;
        let value = load(self, &mut tx, claim, closure_id, retention_operation_id).await?;
        tx.commit().await.map_err(metadata)?;
        Ok(value)
    }
}

async fn load(
    repository: &PostgresReadyRetentionRepository,
    tx: &mut Transaction<'_, Postgres>,
    claim: &OwnedProvisioningClaim,
    closure_id: Uuid,
    retention_operation_id: Uuid,
) -> Result<OwnedRetainedReuseContext, VolumeError> {
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
    sqlx::query("SELECT public.assert_retained_owned_reuse_context($1,$2,$3,$4,$5,$6,$7)")
        .bind(closure_id)
        .bind(claim.operation_id)
        .bind(i64::try_from(claim.generation).map_err(metadata)?)
        .bind(claim.purpose.hash().as_slice())
        .bind(&repository.namespace)
        .bind(&repository.host)
        .bind(retention_operation_id)
        .execute(&mut **tx)
        .await
        .map_err(metadata)?;
    let actual_operation: Uuid = sqlx::query_scalar(
        "SELECT transition.operation_id FROM public.recipe_owned_ready_retentions retained
         JOIN public.recipe_execution_transitions transition ON transition.id=retained.transition_id
         WHERE retained.closure_id=$1 AND transition.kind='creation_retained_ready'",
    )
    .bind(closure_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(metadata)?;
    if actual_operation != retention_operation_id {
        return Err(VolumeError::IntentConflict);
    }
    let result = OwnedRetainedReuseContext {
        retained: context::hydrate(tx, claim, closure_id).await?,
        retention_operation_id: actual_operation,
    };
    result.validate()?;
    Ok(result)
}
