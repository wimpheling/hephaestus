use super::{ReadyRetentionError, retained, validation};
use heph_runtime::VmProvider;
use recipe_application::{OwnedReadyRetentionRepository, ResourceProgress};
use std::sync::{Arc, Weak};
use volume_local::LocalVolumeStore;
use volume_trait::VolumeReadyRetentionRepository;

pub(super) struct Context {
    pub actor: Arc<dyn OwnedReadyRetentionRepository>,
    pub worker: Arc<dyn VolumeReadyRetentionRepository>,
    pub local: Arc<LocalVolumeStore>,
    pub provider: Arc<dyn VmProvider>,
}

pub(super) async fn retain(
    context: &Context,
    request: &retained::Request,
    original: &mut retained::Original,
    registry: &Weak<super::owner::Registry>,
    key: uuid::Uuid,
) -> Result<ResourceProgress, ReadyRetentionError> {
    if request.handover.is_some() && !original.active.exact(request) {
        return super::handover::retain(context, request, original, registry, key).await;
    }
    if !original.active.exact(request)
        || validation::scope(context.provider.as_ref())? != original.request.scope
    {
        return Err(ReadyRetentionError::Correlation);
    }
    request.retention.command.validate(
        &request.identity,
        recipe_application::DeploymentOperation::Remove,
    )?;
    original.identities.push(request.identity.clone());
    // Every retry performs the real current actor qualification before any fact
    // or commit. The stored first identity is comparison data, never credentials.
    let plan = context
        .actor
        .begin_owned_ready_retention(&request.identity, &original.active.retention)
        .await?;
    validation::plan(
        &plan,
        &original.request.claim,
        &original.active.retention.target,
        &original.request.scope,
    )?;
    original.plan(&plan)?;
    let current = context
        .worker
        .ready_retention_context(&original.request.claim, plan.closure_id)
        .await?;
    validation::context(
        &plan,
        &original.request.claim,
        &current,
        &original.request.scope,
    )?;
    if original.custody.is_none() {
        if let Some(progress) = plan.completed {
            // Cold positive replay owns no newly acquired physical guard/proof.
            // No probe, fact record, new operation or actor mutation occurs here.
            if validation::scope(context.provider.as_ref())? != original.request.scope {
                return Err(ReadyRetentionError::Scope);
            }
            return Ok(progress);
        }
        let custody = context
            .local
            .observe_owned_ready_retention(context.worker.as_ref(), &current)
            .await?;
        // SAME guard/context/observation retained BEFORE any awaited fact/commit.
        original.custody = Some(custody);
        super::owner::custody_acquired(registry, key);
    }
    super::completion::commit(context, request, original).await
}
