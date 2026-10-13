use super::{
    Context, RecipeRemovalError, Request, admission,
    retained::{Final, Original},
    validation,
};
use recipe_application::{CommandReceipt, DeploymentLifecycle, DeploymentSnapshot};

pub(super) async fn run(
    context: &Context,
    request: &Request,
    original: &mut Original,
) -> Result<CommandReceipt, RecipeRemovalError> {
    original.request(request)?;
    context.controller.validate_owners()?;
    let snapshot = context
        .deployment
        .inspect(&request.identity, request.remove.deployment_id)
        .await?;
    original.intent(&snapshot.intent)?;
    validation::supported(&snapshot)?;
    if let Some((receipt, before)) = context
        .deployment
        .load_completed_remove(&request.identity, &snapshot.intent)
        .await?
    {
        return restore_completed(context, original, &snapshot, receipt, before);
    }
    if snapshot.lifecycle == DeploymentLifecycle::Removed {
        return Err(RecipeRemovalError::Unsupported);
    }
    let current = admission::admit(context, request, original).await?;
    let order = current.snapshot.intent.execution_order().to_vec();
    for resource in order.into_iter().rev() {
        // Actual ALL-current-rights admission repeats before EACH resource.
        let current = admission::admit(context, request, original).await?;
        let plan = current
            .snapshot
            .intent
            .resources()
            .get(&resource)
            .ok_or(RecipeRemovalError::Unsupported)?;
        if plan.ownership() == recipe_application::ResourceOwnership::External {
            continue;
        }
        match plan.identity() {
            recipe_application::PlannedResourceIdentity::Instance { .. } => {
                super::instances::finish(context, request, original, &current, &resource).await?;
            }
            recipe_application::PlannedResourceIdentity::Volume { .. } => {
                super::volumes::finish(context, request, original, &current, &resource).await?;
            }
        }
    }
    let current = admission::admit(context, request, original).await?;
    validation::complete(&current.snapshot)?;
    context.controller.validate_owners()?;
    let index = if let Some(index) = original
        .finalizations
        .iter()
        .position(|saved| saved.command == request.remove.command)
    {
        index
    } else {
        original.finalizations.push(Final {
            command: request.remove.command,
            before: current.snapshot.version,
            receipt: None,
        });
        original.finalizations.len() - 1
    };
    let before = original.finalizations[index].before; // Never today's Removed version.
    let receipt = context
        .deployment
        .finish_command(
            &request.identity,
            request.remove.command,
            request.remove.deployment_id,
            before,
        )
        .await?;
    original.finalizations[index].receipt = Some(receipt.clone());
    // Positive current-authorized terminal replay verifies exact saved before-CAS,
    // all endpoints/event/result/outbox before returning complete removal.
    let actual = context
        .deployment
        .load_completed_remove(&request.identity, &current.snapshot.intent)
        .await?
        .ok_or(RecipeRemovalError::Unsupported)?;
    if actual != (receipt.clone(), before) {
        return Err(recipe_application::DeploymentError::InputConflict.into());
    }
    context.controller.validate_owners()?;
    Ok(receipt)
}

fn restore_completed(
    context: &Context,
    original: &mut Original,
    snapshot: &DeploymentSnapshot,
    receipt: CommandReceipt,
    before: u64,
) -> Result<CommandReceipt, RecipeRemovalError> {
    if snapshot.lifecycle != DeploymentLifecycle::Removed
        || receipt.deployment_id != snapshot.intent.id()
    {
        return Err(RecipeRemovalError::Unsupported);
    }
    if let Some(saved) = original
        .finalizations
        .iter_mut()
        .find(|saved| saved.command == receipt.command)
    {
        if saved.before != before
            || saved
                .receipt
                .as_ref()
                .is_some_and(|prior| prior != &receipt)
        {
            return Err(recipe_application::DeploymentError::InputConflict.into());
        }
        saved.receipt = Some(receipt.clone());
    } else {
        original.finalizations.push(Final {
            command: receipt.command,
            before,
            receipt: Some(receipt.clone()),
        });
    }
    context.controller.validate_owners()?;
    Ok(receipt) // No new Remove, worker effect or historical identity.
}
