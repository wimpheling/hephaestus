//! Current-manager continuation over actual acquired or still-unacquired custody.
use super::{ReadyRetentionError, retained, validation, work::Context};
use recipe_application::{
    EffectClaim, OwnedReadyRemoveResume, OwnedReadyRetentionPlan, PreparedOwnedVolumeCreation,
    RemoveDeployment, ResourceProgress,
};
use std::sync::Weak;
use volume_trait::OwnedReadyRetentionContext;

/// Crate-private Source8/current-progress comparison data; never an IO permit.
#[derive(Clone)]
pub struct Comparison {
    pub remove: RemoveDeployment,
    pub original: EffectClaim,
    pub preparation: PreparedOwnedVolumeCreation,
    pub progress: ResourceProgress,
    pub committed_fact: Option<uuid::Uuid>,
}

pub(super) async fn retain(
    context: &Context,
    request: &retained::Request,
    original: &mut retained::Original,
    registry: &Weak<super::owner::Registry>,
    key: uuid::Uuid,
) -> Result<ResourceProgress, ReadyRetentionError> {
    let compared = request
        .handover
        .as_ref()
        .ok_or(ReadyRetentionError::Correlation)?;
    if !request.handover_from(&original.active)
        || compared.remove.command != request.retention.command
        || compared.remove.deployment_id != request.retention.target.deployment_id
        || compared.original != original.request.retention.target
        || original
            .history
            .iter()
            .any(|old| old.retention.operation_id == request.retention.operation_id)
        || validation::scope(context.provider.as_ref())? != original.request.scope
    {
        return Err(ReadyRetentionError::Correlation);
    }
    request.retention.command.validate(
        &request.identity,
        recipe_application::DeploymentOperation::Remove,
    )?;
    preparation(&compared.preparation, &original.request.claim)?;
    if original
        .plan
        .as_ref()
        .is_some_and(|prior| prior.preparation != compared.preparation)
    {
        return Err(ReadyRetentionError::Correlation);
    }
    if let Some(custody) = original.custody.as_mut() {
        custody.revalidate()?;
    }
    // This real current-authorized actor port has a separate positive Retained
    // path: no impossible new association/begin or second Retain on completion.
    let disposition = context
        .actor
        .resume_owned_ready_remove(&request.identity, &compared.remove, &compared.original)
        .await?;
    match disposition {
        OwnedReadyRemoveResume::Retained(progress) => {
            completed(context, request, original, compared, progress).await
        }
        OwnedReadyRemoveResume::Admitted(admission) => {
            if compared.committed_fact.is_some()
                || admission.receipt.command != request.retention.command
                || admission.snapshot.version != request.retention.expected_deployment_version
                || admission
                    .snapshot
                    .resources
                    .get(&compared.original.resource)
                    != Some(&compared.progress)
                || compared.progress.version != request.retention.expected_resource_version
            {
                return Err(ReadyRetentionError::Correlation);
            }
            let plan = context
                .actor
                .begin_owned_ready_retention(&request.identity, &request.retention)
                .await?;
            validation::plan(
                &plan,
                &request.claim,
                &request.retention.target,
                &request.scope,
            )?;
            if plan.preparation != compared.preparation || plan.completed.is_some() {
                return Err(ReadyRetentionError::Correlation);
            }
            original.plan(&plan)?;
            let observed = compare_context(context, original, &plan).await?;
            if original.custody.is_none() {
                // The serialized owner has settled all earlier A work. None is
                // only an unacquired guard, never proof of NotApplied/absence.
                let custody = context
                    .local
                    .observe_owned_ready_retention(context.worker.as_ref(), &observed)
                    .await?;
                // Retain the actual guard BEFORE any worker fact/actor wait.
                original.custody = Some(custody);
                super::owner::custody_acquired(registry, key);
            }
            compare_context(context, original, &plan).await?;
            // Current B request becomes active ONLY after actual admission/begin
            // and worker checks. Old request/operation/proof/history stay retained.
            original.history.push(original.active.clone());
            original.active = request.clone();
            original.identities.push(request.identity.clone());
            super::completion::commit(context, request, original).await
        }
    }
}
async fn compare_context(
    context: &Context,
    original: &mut retained::Original,
    plan: &OwnedReadyRetentionPlan,
) -> Result<OwnedReadyRetentionContext, ReadyRetentionError> {
    let observed = context
        .worker
        .ready_retention_context(&original.request.claim, plan.closure_id)
        .await?;
    validation::context(
        plan,
        &original.request.claim,
        &observed,
        &original.request.scope,
    )?;
    if validation::scope(context.provider.as_ref())? != original.request.scope {
        return Err(ReadyRetentionError::Scope);
    }
    if let Some(custody) = original.custody.as_mut() {
        if observed != *custody.context() {
            return Err(ReadyRetentionError::Correlation);
        }
        custody.revalidate()?;
    }
    Ok(observed)
}

fn preparation(
    actual: &PreparedOwnedVolumeCreation,
    claim: &volume_trait::OwnedProvisioningClaim,
) -> Result<(), ReadyRetentionError> {
    let registration = claim.purpose.receipt().intent.registration();
    if actual.mapping_version != 1
        || actual.creation != *claim.purpose.receipt().intent.creation()
        || actual.volume_id != registration.id()
        || actual.project_id != registration.project_id()
        || actual.capacity_bytes != registration.capacity_bytes()
        || actual.filesystem_uuid != registration.filesystem_uuid()
    {
        return Err(ReadyRetentionError::Correlation);
    }
    Ok(())
}

async fn completed(
    context: &Context,
    request: &retained::Request,
    original: &mut retained::Original,
    compared: &Comparison,
    progress: ResourceProgress,
) -> Result<ResourceProgress, ReadyRetentionError> {
    let bound = compared
        .committed_fact
        .ok_or(ReadyRetentionError::Correlation)?;
    if bound.is_nil()
        || progress != compared.progress
        || original
            .committed
            .as_ref()
            .is_some_and(|saved| saved != &progress)
    {
        return Err(ReadyRetentionError::Correlation);
    }
    if let Some(plan) = original.plan.clone() {
        compare_context(context, original, &plan).await?;
    }
    if original.custody.is_none() {
        // Current actor positive Retained + outer Source8 linked fact/full
        // original/preparation + actual worker Original discovery are comparison
        // replay. No guard is invented, no begin/probe/fact/Retain runs here.
        if original.fact.as_ref().is_some_and(|fact| fact.id != bound)
            || validation::scope(context.provider.as_ref())? != original.request.scope
        {
            return Err(ReadyRetentionError::Correlation);
        }
        switch(original, request, progress.clone());
        return Ok(progress);
    }
    // A genuinely held guard must still have its original qualified plan.
    if original.plan.is_none() {
        return Err(ReadyRetentionError::Correlation);
    }
    let custody = original
        .custody
        .as_mut()
        .ok_or(ReadyRetentionError::Correlation)?;
    // The actual committed link was loaded under CURRENT full Source8 rights.
    // Check BEFORE record: this can only replay that existing held observation.
    if bound != custody.observation().fields().id {
        return Err(ReadyRetentionError::Correlation);
    }
    let fact = context
        .worker
        .record_ready_retention_observation(custody.context(), custody.observation())
        .await?;
    if fact.id != bound
        || fact.closure_id != custody.context().closure.fields().id
        || fact.observation != *custody.observation()
        || original.fact.as_ref().is_some_and(|saved| saved != &fact)
    {
        return Err(ReadyRetentionError::Correlation);
    }
    custody.revalidate()?;
    if validation::scope(context.provider.as_ref())? != original.request.scope {
        return Err(ReadyRetentionError::Scope);
    }
    original.fact = Some(fact);
    switch(original, request, progress.clone());
    drop(original.custody.take());
    Ok(progress)
}

fn switch(
    original: &mut retained::Original,
    request: &retained::Request,
    progress: ResourceProgress,
) {
    original.committed = Some(progress);
    original.history.push(original.active.clone());
    original.active = request.clone();
    original.identities.push(request.identity.clone());
}

#[cfg(test)]
#[path = "handover/tests.rs"]
mod tests;
