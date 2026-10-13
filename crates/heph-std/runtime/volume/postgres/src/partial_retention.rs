//! Worker-only positive partial receipts; actual custody remains with the caller.

mod receipts;
mod storage;

use crate::{PostgresVolumeMetadataRepository, errors::metadata};
use async_trait::async_trait;
use volume_trait::{
    OwnedPartialBirthObservation, OwnedPartialRetentionContext, OwnedPartialRetentionReceipt,
    OwnedProvisioningClaim, VolumeError, VolumePartialRetentionRepository,
};

#[async_trait]
impl VolumePartialRetentionRepository for PostgresVolumeMetadataRepository {
    async fn partial_retention_context(
        &self,
        claim: &OwnedProvisioningClaim,
    ) -> Result<OwnedPartialRetentionContext, VolumeError> {
        let mut tx = self.pool.begin().await.map_err(metadata)?;
        storage::exact_partial(&mut tx, claim).await?;
        let head = storage::head(&mut tx, claim).await?;
        tx.commit().await.map_err(metadata)?;
        Ok(OwnedPartialRetentionContext {
            claim: claim.clone(),
            head,
        })
    }
    async fn record_partial_retention_observation(
        &self,
        context: &OwnedPartialRetentionContext,
        observation: &OwnedPartialBirthObservation,
    ) -> Result<OwnedPartialRetentionReceipt, VolumeError> {
        receipts::record(self, context, observation).await
    }
}
