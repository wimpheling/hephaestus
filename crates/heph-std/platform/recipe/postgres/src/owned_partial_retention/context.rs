use crate::{
    PostgresDeploymentRepository,
    effect_context::{self, Context},
    effect_outcome,
    effect_state::State,
    hydration, persistence, receipts, repository_error,
};
use authz_domain::{ObjectRef, ObjectType, Permission};
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, DeploymentLifecycle, DeploymentOperation, DeploymentSnapshot, InstallProgress,
    PlannedResourceIdentity, RemovalProgress, ResourceAction, ResourceOwnership,
    RetainPartialCreation,
};
use recipe_domain::RemovalPolicy;

pub async fn load<'a>(
    repository: &'a PostgresDeploymentRepository,
    identity: &AuthenticatedIdentity,
    request: &RetainPartialCreation,
) -> Result<Context<'a>, DeploymentError> {
    request
        .command
        .validate(identity, DeploymentOperation::Remove)?;
    if request.operation_id == request.target.provenance.attempt_id {
        return Err(DeploymentError::InputConflict);
    }
    let mut tx = begin_actor_transaction(&repository.pool, identity)
        .await
        .map_err(repository_error)?;
    let project = persistence::project(&mut tx, request.target.deployment_id).await?;
    repository
        .require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, project.as_uuid()),
        )
        .await?;
    let PlannedResourceIdentity::Volume {
        id,
        filesystem_uuid: Some(_),
    } = request.target.identity
    else {
        return Err(DeploymentError::InvalidAction);
    };
    repository
        .require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::StateVolume, id.as_uuid()),
        )
        .await?;
    effect_outcome::exact(&mut tx, &request.target).await?;
    // Consumer -> Run -> Volume locks precede project/event scope locks. The
    // helper rechecks both live manager decisions after the quiescence wait.
    sqlx::query("SELECT lock_recipe_partial_retention_volume($1,$2)")
        .bind(id.as_uuid())
        .bind(project.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(repository_error)?;
    repository
        .require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, project.as_uuid()),
        )
        .await?;
    repository
        .require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::StateVolume, id.as_uuid()),
        )
        .await?;
    let organization = persistence::lock_project(&mut tx, project).await?;
    let locked: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT id FROM recipe_deployments WHERE id=$1 FOR UPDATE")
            .bind(request.target.deployment_id.as_uuid())
            .fetch_optional(&mut *tx)
            .await
            .map_err(repository_error)?;
    if locked.is_none() {
        return Err(DeploymentError::Unavailable);
    }
    let snapshot = hydration::load(&mut tx, request.target.deployment_id).await?;
    repository
        .cleanup_authority(&mut tx, identity, &snapshot.intent)
        .await?;
    let row = receipts::find(&mut tx, request.command)
        .await?
        .ok_or(DeploymentError::Unavailable)?;
    if row.id != request.command.id().as_uuid()
        || row.deployment_id != request.target.deployment_id.as_uuid()
    {
        return Err(DeploymentError::InputConflict);
    }
    validate_scope(&snapshot, request)?;
    Ok(Context {
        tx,
        snapshot,
        organization,
        cleanup: true,
        command: request.command,
    })
}

fn validate_scope(
    snapshot: &DeploymentSnapshot,
    request: &RetainPartialCreation,
) -> Result<(), DeploymentError> {
    let plan = snapshot
        .intent
        .resources()
        .get(&request.target.resource)
        .ok_or(DeploymentError::Unavailable)?;
    if !matches!(
        snapshot.lifecycle,
        DeploymentLifecycle::Removing
            | DeploymentLifecycle::RecoveryRequired
            | DeploymentLifecycle::Removed
    ) || request.target.command.operation() != DeploymentOperation::Install
        || request.target.action != ResourceAction::Create
        || plan.ownership() != ResourceOwnership::Owned
        || plan.removal() != RemovalPolicy::Retain
        || plan.identity() != request.target.identity
        || plan.input_hash() != request.target.input_hash
    {
        return Err(DeploymentError::InvalidAction);
    }
    Ok(())
}

pub async fn check_current(
    context: &mut Context<'_>,
    request: &RetainPartialCreation,
) -> Result<State, DeploymentError> {
    let before = effect_context::state(
        &mut context.tx,
        request.target.deployment_id,
        &request.target.resource,
    )
    .await?;
    if context.snapshot.version != request.expected_deployment_version
        || hydration::unsigned(before.version)? != request.expected_resource_version
        || before.active_attempt != Some(request.target.provenance.attempt_id.as_uuid())
        || hydration::unsigned(before.generation)? != request.target.generation
    {
        return Err(DeploymentError::StaleClaim);
    }
    if before.install == InstallProgress::Ready
        || matches!(
            before.removal,
            RemovalProgress::Retained | RemovalProgress::Deleted
        )
        || context.snapshot.lifecycle == DeploymentLifecycle::Removed
    {
        return Err(DeploymentError::InvalidAction);
    }
    let used:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recipe_effect_attempts WHERE id=$1) OR EXISTS(SELECT 1 FROM recipe_effect_verifications WHERE verification_attempt_id=$1) OR EXISTS(SELECT 1 FROM recipe_execution_transitions WHERE operation_id=$1)")
        .bind(request.operation_id.as_uuid()).fetch_one(&mut *context.tx).await.map_err(repository_error)?;
    if used {
        return Err(DeploymentError::InputConflict);
    }
    Ok(before)
}
