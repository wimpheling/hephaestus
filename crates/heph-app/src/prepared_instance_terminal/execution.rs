//! No terminal receipt before independently qualified predecessor/post-state.
use super::{
    Context, PreparedInstanceTerminalError, Request,
    admission::{self, Admission},
    completion,
    retained::Original,
};
use recipe_application::{
    DeploymentError, PreparedInstanceDeleteHistoryState, PreparedInstanceDeletePublication,
    PreparedInstanceDeleteRequest, PreparedInstanceTerminalRepository, ResourceProgress,
};

pub(super) async fn finish(
    context: &Context,
    request: &Request,
    original: &mut Original,
) -> Result<ResourceProgress, PreparedInstanceTerminalError> {
    // At most one Detach and one Delete; no automatic Drain/redelivery loop.
    for _ in 0..2 {
        match Box::pin(admission::prepare(context, request, original)).await? {
            Admission::Completed(checked) => return completed(context, original, &checked).await,
            Admission::Detach(checked) => {
                context.controller.validate_owners()?;
                let observed = context.terminal.observe_detach(&checked).await?;
                if observed.request() != checked.as_ref()
                    || observed.safety().target != *checked.target()
                    || observed.safety().removal_id != Some(checked.closure().removal_id)
                    || observed.safety().state
                        != recipe_application::PreparedInstanceSafetyState::Drained
                {
                    return Err(DeploymentError::IntentMismatch.into());
                }
                // Preserve complete inventory, not merely a Drained boolean.
                if !original.detach_observations.contains(&observed) {
                    original.detach_observations.push(observed);
                }
                context.controller.validate_owners()?;
                completion::commit(context, request, original, checked.effect()).await?;
            }
            Admission::Delete(checked) => {
                return delete(context, request, original, &checked).await;
            }
        }
    }
    Err(DeploymentError::ReconciliationRequired.into())
}
async fn delete(
    context: &Context,
    request: &Request,
    original: &mut Original,
    checked: &PreparedInstanceDeleteRequest,
) -> Result<ResourceProgress, PreparedInstanceTerminalError> {
    context.controller.validate_owners()?;
    let history = context
        .history
        .load_delete_continuation(&request.identity, checked)
        .await?;
    if !super::retained::same_delete(history.original().original(), checked)
        || history.original().original().current_command() != request.remove.command
    {
        return Err(DeploymentError::IntentMismatch.into());
    }
    original.history(&history)?;
    match plan(&history) {
        DeletePlan::Completed => return completed(context, original, checked).await,
        DeletePlan::Publish => {
            context.controller.validate_owners()?;
            // Full request/CAS already retained. Unknown commit never rebases it.
            let published = context
                .terminal
                .delete_for_remove(&request.identity, checked)
                .await?;
            if published.request() != checked {
                return Err(DeploymentError::IntentMismatch.into());
            }
            original.publication(&published, false)?;
        }
        DeletePlan::Observe => {}
    }
    context.controller.validate_owners()?;
    let observed = context.terminal.observe_deleted(checked).await?;
    if observed.request() != checked {
        return Err(DeploymentError::IntentMismatch.into());
    }
    original.publication(&observed, true)?;
    // Independent worker observation, then exact worker recording and current actor.
    context.controller.validate_owners()?;
    completion::commit(context, request, original, checked.effect()).await
}
async fn completed(
    context: &Context,
    original: &mut Original,
    checked: &PreparedInstanceDeleteRequest,
) -> Result<ResourceProgress, PreparedInstanceTerminalError> {
    context.controller.validate_owners()?;
    let observed = context.history.observe_completed_delete(checked).await?;
    if !super::retained::same_delete(observed.original().original(), checked)
        || observed.original().original().current_command() != checked.current_command()
    {
        return Err(DeploymentError::IntentMismatch.into());
    }
    let progress = observed.observed_progress().progress().clone();
    // The configured worker port checks full actual Applied ledger/proof/event.
    // No claim, tombstone, dispatch, verification or actor completion on this path.
    original.delete(checked)?;
    original
        .history(&recipe_application::PreparedInstanceDeleteHistory::completed(observed.clone()))?;
    if !original.completed.contains(&observed) {
        original.completed.push(observed);
    }
    context.controller.validate_owners()?;
    Ok(progress)
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum DeletePlan {
    Publish,
    Observe,
    Completed,
}
// Only used after an actual current-authorized full history read and comparison.
pub(super) fn plan(history: &recipe_application::PreparedInstanceDeleteHistory) -> DeletePlan {
    if history.state() == PreparedInstanceDeleteHistoryState::Applied {
        return DeletePlan::Completed;
    }
    match history.original().publication() {
        PreparedInstanceDeletePublication::Unwritten => DeletePlan::Publish,
        PreparedInstanceDeletePublication::Committed(_) => DeletePlan::Observe,
    }
}
