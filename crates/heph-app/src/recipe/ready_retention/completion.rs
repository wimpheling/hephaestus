use super::{ReadyRetentionError, retained, validation, work::Context};
use recipe_application::ResourceProgress;
use std::sync::Arc;

pub async fn commit(
    context: &Context,
    current: &retained::Request,
    original: &mut retained::Original,
) -> Result<ResourceProgress, ReadyRetentionError> {
    let custody = original
        .custody
        .as_mut()
        .ok_or(ReadyRetentionError::Correlation)?;
    let scope = validation::scope(context.provider.as_ref())?;
    if scope != original.request.scope
        || custody.context().closure.fields().vm_provider_namespace != scope.0
        || custody.context().closure.fields().vm_host_id != scope.1
    {
        return Err(ReadyRetentionError::Scope);
    }
    custody.revalidate()?;
    let observed = context
        .worker
        .ready_retention_context(
            &custody.context().original.claim,
            custody.context().closure.fields().id,
        )
        .await?;
    if observed != *custody.context() {
        return Err(ReadyRetentionError::Correlation);
    }
    custody.revalidate()?;
    if validation::scope(context.provider.as_ref())? != original.request.scope {
        return Err(ReadyRetentionError::Scope);
    }
    let fact = context
        .worker
        .record_ready_retention_observation(custody.context(), custody.observation())
        .await?;
    if fact.id != custody.observation().fields().id
        || fact.closure_id != custody.context().closure.fields().id
        || fact.observation != *custody.observation()
        || original.fact.as_ref().is_some_and(|saved| saved != &fact)
    {
        return Err(ReadyRetentionError::Correlation);
    }
    original.fact = Some(fact.clone());
    custody.revalidate()?;
    if validation::scope(context.provider.as_ref())? != original.request.scope {
        return Err(ReadyRetentionError::Scope);
    }
    let progress = context
        .actor
        .retain_owned_ready(&current.identity, &original.active.retention, fact.id)
        .await?;
    if original
        .committed
        .as_ref()
        .is_some_and(|saved| saved != &progress)
    {
        return Err(ReadyRetentionError::Correlation);
    }
    original.committed = Some(progress.clone());
    if let Err(error) = custody.revalidate() {
        original
            .errors
            .push(Arc::new(ReadyRetentionError::Volume(error)));
        return Err(ReadyRetentionError::CommittedCustodyChanged { progress });
    }
    // A fresh current-authorized durable completed replay is REQUIRED before
    // releasing the acquired guard, even after a successful first commit reply.
    let completed = context
        .actor
        .begin_owned_ready_retention(&current.identity, &original.active.retention)
        .await?;
    validation::plan(
        &completed,
        &original.request.claim,
        &original.active.retention.target,
        &scope,
    )?;
    if completed.completed.as_ref() != Some(&progress)
        || original.plan.as_ref().is_none_or(|plan| {
            plan.preparation != completed.preparation
                || plan.closure_id != completed.closure_id
                || plan.vm_provider_namespace != completed.vm_provider_namespace
                || plan.vm_host_id != completed.vm_host_id
        })
    {
        return Err(ReadyRetentionError::Correlation);
    }
    let observed = context
        .worker
        .ready_retention_context(&original.request.claim, completed.closure_id)
        .await?;
    if observed != *custody.context()
        || validation::scope(context.provider.as_ref())? != original.request.scope
    {
        return Err(ReadyRetentionError::Correlation);
    }
    if let Err(error) = custody.revalidate() {
        original
            .errors
            .push(Arc::new(ReadyRetentionError::Volume(error)));
        return Err(ReadyRetentionError::CommittedCustodyChanged { progress });
    }
    drop(original.custody.take());
    Ok(progress)
}
