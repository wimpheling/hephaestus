use super::{Context, Mode, RecipeRemovalError, Request, retained::Original, validation};
use recipe_application::{
    DeploymentExecutionPreparationRepository, DeploymentSnapshot, OriginalInstanceClosure,
};

pub(super) struct Current {
    pub snapshot: DeploymentSnapshot,
    pub closures: Vec<OriginalInstanceClosure>,
}
pub(super) async fn admit(
    context: &Context,
    request: &Request,
    original: &mut Original,
) -> Result<Current, RecipeRemovalError> {
    original.request(request)?;
    context.controller.validate_owners()?;
    request.remove.command.validate(
        &request.identity,
        recipe_application::DeploymentOperation::Remove,
    )?;
    let current = match request.mode {
        Mode::Original => {
            let snapshot = context
                .deployment
                .admit_remove(&request.identity, request.remove)
                .await?
                .snapshot;
            let closures = context
                .deployment
                .load_remove_original_closures(
                    &request.identity,
                    request.remove.command,
                    &snapshot.intent,
                )
                .await?;
            Current { snapshot, closures }
        }
        Mode::Resume => {
            let current = context
                .deployment
                .admit_remove_resume(&request.identity, request.remove)
                .await?;
            Current {
                snapshot: current.admission().snapshot.clone(),
                closures: current.closures().to_vec(),
            }
        }
    };
    original.intent(&current.snapshot.intent)?;
    validation::supported(&current.snapshot)?;
    let prepared = context
        .preparation
        .inspect_execution_preparation(&request.identity, request.remove.deployment_id)
        .await?
        .ok_or(RecipeRemovalError::Unsupported)?;
    let cfg = &context.controller.drain_inputs().configuration;
    let platform = recipe_application::PlatformPolicyObservation::new(
        cfg.platform_policy().clone(),
        cfg.platform_version().to_owned(),
    )?;
    prepared.validate_configuration(
        recipe_application::DeploymentExecutionProfile::RuntimeNamedV1,
        &platform,
    )?;
    context.controller.validate_owners()?;
    Ok(current)
}
