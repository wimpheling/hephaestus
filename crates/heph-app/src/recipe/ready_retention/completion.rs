use super::{ReadyRetentionError, validation};
use heph_runtime::VmProvider;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{OwnedReadyRetentionRepository, ResourceProgress, RetainOwnedReady};
use std::sync::Arc;
use volume_local::OwnedReadyRetentionCustody;
use volume_trait::VolumeReadyRetentionRepository;

pub async fn commit(
    actor: Arc<dyn OwnedReadyRetentionRepository>,
    worker: Arc<dyn VolumeReadyRetentionRepository>,
    provider: Arc<dyn VmProvider>,
    identity: AuthenticatedIdentity,
    request: RetainOwnedReady,
    mut custody: OwnedReadyRetentionCustody,
) -> Result<ResourceProgress, ReadyRetentionError> {
    let scope = validation::scope(provider.as_ref())?;
    if custody.context().closure.fields().vm_provider_namespace != scope.0
        || custody.context().closure.fields().vm_host_id != scope.1
    {
        return Err(ReadyRetentionError::Scope);
    }
    custody.revalidate()?;
    let current = worker
        .ready_retention_context(
            &custody.context().original.claim,
            custody.context().closure.fields().id,
        )
        .await?;
    if current != *custody.context() {
        return Err(ReadyRetentionError::Correlation);
    }
    let fact = worker
        .record_ready_retention_observation(custody.context(), custody.observation())
        .await?;
    if fact.id != custody.observation().fields().id
        || fact.closure_id != custody.context().closure.fields().id
        || fact.observation != *custody.observation()
    {
        return Err(ReadyRetentionError::Correlation);
    }
    custody.revalidate()?;
    let progress = actor
        .retain_owned_ready(&identity, &request, fact.id)
        .await?;
    if custody.revalidate().is_err() {
        return Err(ReadyRetentionError::CommittedCustodyChanged { progress });
    }
    drop(custody);
    // The actual provider clone remains alive until both awaited commits resolve.
    drop(provider);
    Ok(progress)
}
