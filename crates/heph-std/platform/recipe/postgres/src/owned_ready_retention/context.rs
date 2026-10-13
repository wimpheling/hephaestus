#[path = "context/configured.rs"]
pub mod configured;
pub use configured::original_input;

use super::PostgresOwnedReadyRetentionRepository;
use crate::{
    effect_context::{self, Context},
    effect_outcome,
    effect_state::State,
    hydration, owned_creation_preparation, persistence, receipts, repository_error,
};
use authz_domain::{ObjectRef, ObjectType, Permission};
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, DeploymentLifecycle, DeploymentOperation, PlannedResourceIdentity,
    PreparedOwnedVolumeCreation, RemovalProgress, ResourceAction, ResourceOwnership,
    RetainOwnedReady,
};
use recipe_domain::RemovalPolicy;

pub async fn load<'a>(
    adapter: &'a PostgresOwnedReadyRetentionRepository,
    identity: &AuthenticatedIdentity,
    request: &RetainOwnedReady,
) -> Result<Context<'a>, DeploymentError> {
    if request.operation_id == request.target.provenance.attempt_id {
        return Err(DeploymentError::InvalidAction);
    }
    let mut context = manager(adapter, identity, request.command, &request.target).await?;
    let command = receipts::find(&mut context.tx, request.command)
        .await?
        .ok_or(DeploymentError::Unavailable)?;
    if command.deployment_id != request.target.deployment_id.as_uuid()
        || command.id != request.command.id().as_uuid()
    {
        return Err(DeploymentError::InputConflict);
    }
    let original = original_input(adapter, &mut context, request.command).await?;
    configured::receipt(&command.receipt_lifecycle, &command.input_hash, original)?;
    Ok(context)
}
/// All-owned cleanup permissions are repeated before any fresh resume admission.
pub async fn manager<'a>(
    adapter: &'a PostgresOwnedReadyRetentionRepository,
    identity: &AuthenticatedIdentity,
    command: recipe_application::CommandIdentity,
    target: &recipe_application::EffectClaim,
) -> Result<Context<'a>, DeploymentError> {
    command.validate(identity, DeploymentOperation::Remove)?;
    if target.command.operation() != DeploymentOperation::Install
        || target.action != ResourceAction::Create
    {
        return Err(DeploymentError::InvalidAction);
    }
    let PlannedResourceIdentity::Volume {
        id,
        filesystem_uuid: Some(_),
    } = target.identity
    else {
        return Err(DeploymentError::InvalidAction);
    };
    let mut tx = begin_actor_transaction(&adapter.repository.pool, identity)
        .await
        .map_err(repository_error)?;
    active(&mut tx, identity).await?;
    let project = persistence::project(&mut tx, target.deployment_id).await?;
    permissions(adapter, &mut tx, identity, project, id).await?;
    sqlx::query("SELECT reserve_ready_retention_deployment($1)")
        .bind(target.deployment_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(repository_error)?;
    let organization = persistence::lock_project(&mut tx, project).await?;
    sqlx::query("SELECT id FROM recipe_deployments WHERE id=$1 FOR UPDATE")
        .bind(target.deployment_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(repository_error)?
        .ok_or(DeploymentError::Unavailable)?;
    effect_context::state(&mut tx, target.deployment_id, &target.resource).await?;
    let snapshot = hydration::load(&mut tx, target.deployment_id).await?;
    adapter
        .preparation
        .assert_ready_retention_configuration(&mut tx, &snapshot.intent)
        .await?;
    active(&mut tx, identity).await?;
    adapter
        .repository
        .cleanup_authority(&mut tx, identity, &snapshot.intent)
        .await?;
    permissions(adapter, &mut tx, identity, project, id).await?;
    effect_outcome::exact(&mut tx, target).await?;
    let resource = snapshot
        .intent
        .resources()
        .get(&target.resource)
        .ok_or(DeploymentError::Unavailable)?;
    if !matches!(
        snapshot.lifecycle,
        DeploymentLifecycle::Removing
            | DeploymentLifecycle::RecoveryRequired
            | DeploymentLifecycle::Removed
    ) || resource.ownership() != ResourceOwnership::Owned
        || resource.removal() != RemovalPolicy::Retain
        || resource.identity() != target.identity
        || resource.input_hash() != target.input_hash
    {
        return Err(DeploymentError::InvalidAction);
    }
    Ok(Context {
        tx,
        snapshot,
        organization,
        cleanup: true,
        command,
    })
}
async fn permissions(
    adapter: &PostgresOwnedReadyRetentionRepository,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    identity: &AuthenticatedIdentity,
    project: forge_domain::ProjectId,
    volume: runtime_types::VolumeId,
) -> Result<(), DeploymentError> {
    adapter
        .repository
        .require(
            tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, project.as_uuid()),
        )
        .await?;
    adapter
        .repository
        .require(
            tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::StateVolume, volume.as_uuid()),
        )
        .await
}
pub async fn active(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    identity: &AuthenticatedIdentity,
) -> Result<(), DeploymentError> {
    let allowed:bool=sqlx::query_scalar("SELECT current_user::text='hephaestus_app' AND EXISTS(SELECT 1 FROM users WHERE id=$1 AND status='active')")
        .bind(identity.user_id.as_uuid()).fetch_one(&mut **tx).await.map_err(repository_error)?;
    if allowed {
        Ok(())
    } else {
        Err(DeploymentError::AuthorizationDenied)
    }
}
pub async fn prepared(
    context: &mut Context<'_>,
    target: &recipe_application::EffectClaim,
) -> Result<PreparedOwnedVolumeCreation, DeploymentError> {
    let stored = owned_creation_preparation::storage::load(&mut context.tx, target)
        .await?
        .ok_or(DeploymentError::Unavailable)?;
    if stored != owned_creation_preparation::mapping::derive(&context.snapshot.intent, target)? {
        return Err(DeploymentError::InputConflict);
    }
    Ok(stored)
}
pub async fn current(
    context: &mut Context<'_>,
    request: &RetainOwnedReady,
) -> Result<State, DeploymentError> {
    let before = effect_context::state(
        &mut context.tx,
        request.target.deployment_id,
        &request.target.resource,
    )
    .await?;
    if context.snapshot.version != request.expected_deployment_version
        || hydration::unsigned(before.version)? != request.expected_resource_version
        || hydration::unsigned(before.generation)? != request.target.generation
        || matches!(
            before.removal,
            RemovalProgress::Retained | RemovalProgress::Deleted
        )
        || (before.active_attempt.is_some()
            && before.active_attempt != Some(request.target.provenance.attempt_id.as_uuid()))
    {
        return Err(DeploymentError::StaleClaim);
    }
    Ok(before)
}
