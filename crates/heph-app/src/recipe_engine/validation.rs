use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentExecutionPreparation, DeploymentIntent,
    DeploymentLifecycle, DeploymentSnapshot, InstallProgress, PlannedResourceIdentity,
    ResourceOwnership, ResourceProgress,
};

use super::RecipeApplicationError;

pub fn supported(
    intent: &DeploymentIntent,
    retained_volumes: bool,
) -> Result<(), RecipeApplicationError> {
    for (name, resource) in intent.resources() {
        if resource.ownership() == ResourceOwnership::External
            && (!retained_volumes
                || !matches!(resource.identity(), PlannedResourceIdentity::Volume { .. }))
        {
            return Err(RecipeApplicationError::UnsupportedExternal(name.clone()));
        }
    }
    Ok(())
}

pub fn preparation(
    intent: &DeploymentIntent,
    command: CommandIdentity,
    prepared: &DeploymentExecutionPreparation,
) -> Result<(), DeploymentError> {
    if prepared.command() != command
        || prepared.deployment_id() != intent.id()
        || prepared.project_id() != intent.project_id()
        || prepared.intent_hash() != intent.input_hash()
    {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(())
}

pub fn installing(
    snapshot: &DeploymentSnapshot,
    intent: &DeploymentIntent,
) -> Result<(), DeploymentError> {
    intent.validate_replay(&snapshot.intent)?;
    if !matches!(
        snapshot.lifecycle,
        DeploymentLifecycle::Installing
            | DeploymentLifecycle::Installed
            | DeploymentLifecycle::RecoveryRequired
    ) {
        return Err(DeploymentError::InvalidAction);
    }
    Ok(())
}

pub fn ready(progress: &ResourceProgress) -> Result<(), DeploymentError> {
    if progress.install != InstallProgress::Ready || progress.active_attempt.is_some() {
        return Err(DeploymentError::InvalidAction);
    }
    Ok(())
}
