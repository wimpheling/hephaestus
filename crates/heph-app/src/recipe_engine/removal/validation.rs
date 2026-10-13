//! Pure scheduling/policy comparison only; protected adapters qualify history.
use super::RecipeRemovalError;
use recipe_application::{
    DeploymentSnapshot, InstallProgress, PlannedResourceIdentity, RemovalProgress,
    ResourceOwnership,
};
use recipe_domain::RemovalPolicy;

pub(super) fn supported(snapshot: &DeploymentSnapshot) -> Result<(), RecipeRemovalError> {
    if snapshot.resources.len() != snapshot.intent.resources().len() {
        return Err(recipe_application::DeploymentError::IntentMismatch.into());
    }
    for (name, plan) in snapshot.intent.resources() {
        let state = snapshot
            .resources
            .get(name)
            .ok_or(RecipeRemovalError::Unsupported)?;
        match (plan.ownership(), plan.identity(), plan.removal()) {
            (
                ResourceOwnership::Owned,
                PlannedResourceIdentity::Instance { .. },
                RemovalPolicy::Delete,
            )
            | (
                ResourceOwnership::Owned,
                PlannedResourceIdentity::Volume {
                    filesystem_uuid: Some(_),
                    ..
                },
                RemovalPolicy::Retain,
            ) => {
                if state.install != InstallProgress::Ready {
                    return Err(RecipeRemovalError::Unsupported);
                }
            }
            (ResourceOwnership::External, _, RemovalPolicy::Retain) => {
                if state.active_attempt.is_some() || state.removal != RemovalProgress::Retained {
                    return Err(RecipeRemovalError::Unsupported);
                }
            }
            _ => return Err(RecipeRemovalError::Unsupported),
        }
    }
    Ok(())
}
pub(super) fn complete(snapshot: &DeploymentSnapshot) -> Result<(), RecipeRemovalError> {
    supported(snapshot)?;
    for (name, plan) in snapshot.intent.resources() {
        let state = &snapshot.resources[name];
        let expected = if plan.ownership() == ResourceOwnership::Owned
            && plan.removal() == RemovalPolicy::Delete
        {
            RemovalProgress::Deleted
        } else {
            RemovalProgress::Retained
        };
        if state.active_attempt.is_some() || state.removal != expected {
            return Err(recipe_application::DeploymentError::ReconciliationRequired.into());
        }
    }
    Ok(())
}

/// Routing by an actual loaded original effect, never payload or tombstone alone.
pub(super) fn needs_drain(
    removal: RemovalProgress,
    active: Option<recipe_application::ResourceAction>,
) -> Result<bool, RecipeRemovalError> {
    use recipe_application::ResourceAction;
    match active {
        Some(ResourceAction::Drain) => Ok(true),
        Some(ResourceAction::Detach | ResourceAction::Delete) => Ok(false),
        None if removal == RemovalProgress::Pending => Ok(true),
        None if matches!(
            removal,
            RemovalProgress::Draining | RemovalProgress::Detached | RemovalProgress::Deleted
        ) =>
        {
            Ok(false)
        }
        None | Some(_) => Err(RecipeRemovalError::Unsupported),
    }
}
