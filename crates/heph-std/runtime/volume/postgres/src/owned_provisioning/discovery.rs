//! Worker-only global original-history discovery; no claims or host effects.

mod history;
mod observations;

use volume_trait::{
    OwnedProvisioningDiscovery, OwnedProvisioningExpectation, VolumeCreationLineage, VolumeError,
};

use super::storage;
use crate::{PostgresVolumeMetadataRepository, errors::metadata, owned_registration};

pub(super) async fn inspect(
    repository: &PostgresVolumeMetadataRepository,
    expected: &OwnedProvisioningExpectation,
) -> Result<OwnedProvisioningDiscovery, VolumeError> {
    let mut tx = repository.pool.begin().await.map_err(metadata)?;
    // One consistent snapshot, with no lifecycle writes or resource locks. A
    // concurrent admission cannot turn this observation into admission authority.
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(metadata)?;
    storage::role(&mut tx, "hephaestus_worker").await?;
    let volume_id = expected.intent().registration().id();
    let VolumeCreationLineage::Owned(receipt) =
        owned_registration::lineage(&mut tx, volume_id).await?
    else {
        return Err(VolumeError::IntentConflict);
    };
    let purpose = expected.purpose(*receipt)?;
    storage::original_receipt(&mut tx, &purpose).await?;
    history::birth_event(&mut tx, &purpose).await?;
    storage::require_live(&mut tx, volume_id.as_uuid()).await?;
    let row = history::volume(&mut tx, &purpose).await?;
    // These lookups are global by volume, never filtered by configured host/root.
    let operations = history::operations(&mut tx, volume_id.as_uuid()).await?;
    let result = if operations.is_empty() {
        history::unadmitted(&mut tx, volume_id.as_uuid(), &row).await?;
        OwnedProvisioningDiscovery::Unadmitted(Box::new(purpose.receipt().clone()))
    } else {
        let [operation] = operations.as_slice() else {
            return Err(VolumeError::ProvisioningUncertain(
                "multiple owned operations require held reconciliation",
            ));
        };
        history::purpose(&mut tx, &purpose).await?;
        history::operation(&mut tx, expected, &purpose, operation).await?;
        let claim = operation.claim(&purpose)?;
        let observation = observations::load(&mut tx, &claim).await?;
        row.admitted(&purpose, observation.as_ref())?;
        OwnedProvisioningDiscovery::Original(Box::new(volume_trait::OwnedProvisioningContext {
            claim,
            observation,
        }))
    };
    tx.commit().await.map_err(metadata)?;
    Ok(result)
}
