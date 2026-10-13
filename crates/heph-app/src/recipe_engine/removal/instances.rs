use super::{
    Context, Mode, RecipeRemovalError, Request, admission::Current, retained::Original, validation,
};
use capability_domain::CapabilitySlotKey;
use recipe_application::{
    ActiveDeploymentEffectRepository, PlannedResourceIdentity, RemovalProgress,
};

pub(super) async fn finish(
    context: &Context,
    request: &Request,
    original: &mut Original,
    current: &Current,
    resource: &CapabilitySlotKey,
) -> Result<(), RecipeRemovalError> {
    let plan = current
        .snapshot
        .intent
        .resources()
        .get(resource)
        .ok_or(RecipeRemovalError::Unsupported)?;
    let PlannedResourceIdentity::Instance { id, .. } = plan.identity() else {
        return Err(RecipeRemovalError::Unsupported);
    };
    let state = current
        .snapshot
        .resources
        .get(resource)
        .ok_or(RecipeRemovalError::Unsupported)?;
    let active = context
        .preparation
        .load_active_effect(
            &request.identity,
            request.remove.command,
            request.remove.deployment_id,
            resource,
        )
        .await?;
    if validation::needs_drain(state.removal, active.as_ref().map(|claim| claim.action))? {
        let key = (resource.clone(), request.remove.command.id().as_uuid());
        let cas = if let Some(saved) = original.instance_cas.get(&key) {
            *saved
        } else {
            // An actual current-admission closed original supplies its recorded
            // before-CAS. An unpublished target uses the authorized Release query.
            let recorded = current
                .closures
                .iter()
                .find(|closure| closure.drain.resource == *resource)
                .map(|closure| closure.expected_version);
            let cas = if let Some(recorded) = recorded {
                recorded
            } else {
                let actual = context
                    .release
                    .instance_volume_status(&request.identity, id)
                    .await?;
                if actual.instance_id != id {
                    return Err(RecipeRemovalError::Unsupported);
                }
                actual.version
            };
            if cas == 0 || cas > i64::MAX.cast_unsigned() {
                return Err(RecipeRemovalError::Unsupported);
            }
            original.instance_cas.insert(key, cas); // BEFORE any closure commit wait.
            cas
        };
        context.controller.validate_owners()?;
        let drained = match request.mode {
            Mode::Original => {
                context
                    .drain
                    .drain(&request.identity, request.remove, resource, cas)
                    .await
            }
            Mode::Resume => {
                context
                    .drain
                    .resume(&request.identity, request.remove, resource, cas)
                    .await
            }
        }
        .map_err(RecipeRemovalError::Drain)?;
        if drained.active_attempt.is_some() || drained.removal != RemovalProgress::Draining {
            return Err(RecipeRemovalError::Unsupported);
        }
    }
    context.controller.validate_owners()?;
    // Terminal coordinator itself qualifies completed Drain, exact Detach/Delete
    // and full worker history, including positively inactive Applied Delete replay.
    let deleted = match request.mode {
        Mode::Original => {
            context
                .terminal
                .finish(&request.identity, request.remove, resource)
                .await
        }
        Mode::Resume => {
            context
                .terminal
                .resume(&request.identity, request.remove, resource)
                .await
        }
    }
    .map_err(RecipeRemovalError::Terminal)?;
    if deleted.active_attempt.is_some() || deleted.removal != RemovalProgress::Deleted {
        return Err(RecipeRemovalError::Unsupported);
    }
    context.controller.validate_owners()?;
    Ok(())
}
