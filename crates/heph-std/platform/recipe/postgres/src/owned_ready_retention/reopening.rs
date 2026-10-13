use super::{PostgresOwnedReadyRetentionRepository, context};
use crate::{
    effect_context, effect_outcome, execution_rows::ClaimWire, execution_transitions, hydration,
    persistence, repository_error,
};
use authz_domain::{ObjectRef, ObjectType, Permission};
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, DeploymentOperation, PlannedResourceIdentity, ReopenRetainedOwnedVolume,
    ResourceAction, ResourceOwnership,
};
use uuid::Uuid;

pub async fn reopen(
    adapter: &PostgresOwnedReadyRetentionRepository,
    identity: &AuthenticatedIdentity,
    request: &ReopenRetainedOwnedVolume,
) -> Result<(), DeploymentError> {
    let claim = &request.claim;
    claim
        .command
        .validate(identity, DeploymentOperation::Install)?;
    let PlannedResourceIdentity::Volume {
        id,
        filesystem_uuid: _,
    } = claim.identity
    else {
        return Err(DeploymentError::InvalidAction);
    };
    if claim.action != ResourceAction::VerifyExternal {
        return Err(DeploymentError::InvalidAction);
    }
    let (mut current, state) = current_context(adapter, identity, claim, id).await?;
    let tx = &mut current.tx;
    let project = current.snapshot.intent.project_id();
    let closure:Option<Uuid>=sqlx::query_scalar("SELECT closure.id FROM volume_owned_ready_retention_closures closure
        JOIN recipe_owned_ready_retentions retained ON retained.closure_id=closure.id
        JOIN recipe_execution_transitions transition ON transition.id=retained.transition_id
        WHERE closure.volume_id=$1 AND closure.project_id=$2 AND closure.vm_provider_namespace=$3 AND closure.vm_host_id=$4
          AND transition.kind='creation_retained_ready' AND transition.operation_id=$5")
        .bind(id.as_uuid()).bind(project.as_uuid()).bind(&adapter.namespace).bind(&adapter.host)
        .bind(request.retention_operation_id.as_uuid()).fetch_optional(&mut **tx).await.map_err(repository_error)?;
    let closure = closure.ok_or(DeploymentError::ReconciliationRequired)?;
    let operation = Uuid::new_v5(
        &claim.provenance.attempt_id.as_uuid(),
        b"heph-owned-ready-reopening-v1\0",
    );
    let json =
        serde_json::to_value(ClaimWire::new(claim)?).map_err(|_| DeploymentError::Serialization)?;
    let existing: Option<(Uuid, serde_json::Value)> = sqlx::query_as(
        "SELECT closure_id,claim_json FROM recipe_owned_ready_reopenings WHERE id=$1",
    )
    .bind(operation)
    .fetch_optional(&mut **tx)
    .await
    .map_err(repository_error)?;
    if let Some((old, stored)) = existing {
        if old != closure || stored != json {
            return Err(DeploymentError::InputConflict);
        }
        current.tx.commit().await.map_err(repository_error)?;
        return Ok(());
    }
    if state.active_attempt != Some(claim.provenance.attempt_id.as_uuid()) {
        return Err(DeploymentError::StaleClaim);
    }
    append(&mut current, identity, claim, closure, operation, json).await?;
    current.tx.commit().await.map_err(repository_error)
}

pub(super) async fn current_context<'a>(
    adapter: &'a PostgresOwnedReadyRetentionRepository,
    identity: &AuthenticatedIdentity,
    claim: &recipe_application::EffectClaim,
    id: runtime_types::VolumeId,
) -> Result<
    (
        crate::effect_context::Context<'a>,
        crate::effect_state::State,
    ),
    DeploymentError,
> {
    let mut tx = begin_actor_transaction(&adapter.repository.pool, identity)
        .await
        .map_err(repository_error)?;
    context::active(&mut tx, identity).await?;
    let project = persistence::project(&mut tx, claim.deployment_id).await?;
    adapter
        .repository
        .require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, project.as_uuid()),
        )
        .await?;
    adapter
        .repository
        .require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::StateVolume, id.as_uuid()),
        )
        .await?;
    sqlx::query("SELECT reserve_ready_retention_deployment($1)")
        .bind(claim.deployment_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(repository_error)?;
    let organization = persistence::lock_project(&mut tx, project).await?;
    sqlx::query("SELECT id FROM recipe_deployments WHERE id=$1 FOR UPDATE")
        .bind(claim.deployment_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(repository_error)?
        .ok_or(DeploymentError::Unavailable)?;
    let state = effect_context::state(&mut tx, claim.deployment_id, &claim.resource).await?;
    let snapshot = hydration::load(&mut tx, claim.deployment_id).await?;
    adapter
        .preparation
        .assert_current_reopening(&mut tx, identity, claim.command, &snapshot)
        .await?;
    context::active(&mut tx, identity).await?;
    adapter
        .repository
        .require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::StateVolume, id.as_uuid()),
        )
        .await?;
    effect_outcome::exact(&mut tx, claim).await?;
    let plan = snapshot
        .intent
        .resources()
        .get(&claim.resource)
        .ok_or(DeploymentError::Unavailable)?;
    if plan.ownership() != ResourceOwnership::External
        || hydration::unsigned(state.generation)? != claim.generation
    {
        return Err(DeploymentError::StaleClaim);
    }
    Ok((
        crate::effect_context::Context {
            tx,
            snapshot,
            organization,
            cleanup: false,
            command: claim.command,
        },
        state,
    ))
}

pub(super) async fn append(
    current: &mut effect_context::Context<'_>,
    identity: &AuthenticatedIdentity,
    claim: &recipe_application::EffectClaim,
    closure: Uuid,
    operation: Uuid,
    json: serde_json::Value,
) -> Result<(), DeploymentError> {
    let tx = &mut current.tx;
    let project = current.snapshot.intent.project_id();
    let (event, _, _) =
        execution_transitions::event(tx, identity, operation, current.organization, project)
            .await?;
    sqlx::query("INSERT INTO recipe_owned_ready_reopenings(id,closure_id,attempt_id,deployment_id,project_id,resource_name,command_id,actor_id,request_id,claim_json,event_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
        .bind(operation).bind(closure).bind(claim.provenance.attempt_id.as_uuid()).bind(claim.deployment_id.as_uuid())
        .bind(project.as_uuid()).bind(claim.resource.as_str()).bind(claim.command.id().as_uuid())
        .bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid()).bind(json).bind(event)
        .execute(&mut **tx).await.map_err(repository_error)?;
    Ok(())
}
