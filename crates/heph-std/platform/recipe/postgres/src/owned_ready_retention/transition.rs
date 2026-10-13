use crate::{
    effect_context::Context, effect_state::State, execution_transitions, repository_error,
};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, DeploymentLifecycle, InstallProgress, RemovalProgress, ResourceAction,
    ResourceProgress, RetainOwnedReady,
};
use release_domain::ContentHash;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub fn id(request: &RetainOwnedReady) -> Uuid {
    execution_transitions::transition_id(request.operation_id.as_uuid(), "creation_retained_ready")
}
pub fn fingerprint(request: &RetainOwnedReady) -> Result<ContentHash, DeploymentError> {
    execution_transitions::fingerprint(
        &request.target,
        "creation_retained_ready",
        &serde_json::json!([
            request.command,
            request.operation_id,
            request.expected_deployment_version,
            request.expected_resource_version
        ]),
    )
}
pub async fn completed(
    context: &mut Context<'_>,
    request: &RetainOwnedReady,
    closure: Uuid,
) -> Result<Option<ResourceProgress>, DeploymentError> {
    let result:Option<sqlx::types::Json<State>>=sqlx::query_scalar(
        "SELECT transition.after_state FROM recipe_owned_ready_retentions retained
         JOIN recipe_execution_transitions transition ON transition.id=retained.transition_id AND transition.kind='creation_retained_ready'
         JOIN recipe_deployment_resources resource ON resource.deployment_id=transition.deployment_id AND resource.resource_name=transition.resource_name
         JOIN agent_instance_state_volumes volume ON volume.id=resource.resource_id
         WHERE retained.closure_id=$1 AND transition.attempt_id=$2
           AND transition.deployment_id=$3 AND transition.resource_name=$4
           AND resource.install_progress='ready' AND resource.removal_progress='retained' AND resource.active_attempt_id IS NULL
           AND recipe_effect_resource_state(resource)=transition.after_state AND volume.ready_retention_closure_id=$1
")
        .bind(closure).bind(request.target.provenance.attempt_id.as_uuid()).bind(request.target.deployment_id.as_uuid())
        .bind(request.target.resource.as_str()).fetch_optional(&mut *context.tx).await.map_err(repository_error)?;
    if let Some(state) =
        execution_transitions::replay(&mut context.tx, id(request), fingerprint(request)?).await?
    {
        if result.as_ref().map(|row| &row.0) != Some(&state) {
            return Err(DeploymentError::InputConflict);
        }
    }
    result.map(|state| state.0.progress()).transpose()
}
pub async fn exact_fact(
    tx: &mut Transaction<'_, Postgres>,
    request: &RetainOwnedReady,
    fact: Uuid,
    closure: Uuid,
) -> Result<(), DeploymentError> {
    let exact:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recipe_owned_ready_retentions retained
        JOIN recipe_execution_transitions transition ON transition.id=retained.transition_id
        WHERE retained.closure_id=$1 AND retained.fact_id=$2 AND transition.id=$3 AND transition.ready_retention_fact_id=$2)")
        .bind(closure).bind(fact).bind(id(request)).fetch_one(&mut **tx).await.map_err(repository_error)?;
    if exact {
        Ok(())
    } else {
        Err(DeploymentError::InputConflict)
    }
}
pub async fn commit(
    context: &mut Context<'_>,
    identity: &AuthenticatedIdentity,
    request: &RetainOwnedReady,
    before: &State,
    closure: Uuid,
    fact: Uuid,
) -> Result<State, DeploymentError> {
    let mut after = before.clone();
    after.install = InstallProgress::Ready;
    after.removal = RemovalProgress::Retained;
    after.active_attempt = None;
    after.diagnostic = None;
    after.last_action = Some(ResourceAction::Retain);
    after.version = before
        .version
        .checked_add(1)
        .ok_or(DeploymentError::StaleClaim)?;
    let after_version = context
        .snapshot
        .version
        .checked_add(1)
        .ok_or(DeploymentError::StaleClaim)?;
    let uncertain:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recipe_deployment_resources WHERE deployment_id=$1 AND resource_name<>$2 AND active_attempt_id IS NOT NULL AND (install_progress IN ('failed','recovery_required') OR removal_progress='recovery_required'))")
        .bind(request.target.deployment_id.as_uuid()).bind(request.target.resource.as_str()).fetch_one(&mut *context.tx).await.map_err(repository_error)?;
    let lifecycle = if uncertain {
        DeploymentLifecycle::RecoveryRequired
    } else {
        DeploymentLifecycle::Removing
    };
    let (event, cursor, version) = execution_transitions::event(
        &mut context.tx,
        identity,
        id(request),
        context.organization,
        context.snapshot.intent.project_id(),
    )
    .await?;
    sqlx::query("INSERT INTO recipe_owned_ready_retentions(closure_id,fact_id,transition_id) VALUES($1,$2,$3)")
        .bind(closure).bind(fact).bind(id(request)).execute(&mut *context.tx).await.map_err(repository_error)?;
    sqlx::query("INSERT INTO recipe_execution_transitions(id,command_id,attempt_id,operation_id,deployment_id,project_id,resource_name,kind,fingerprint,actor_id,request_id,before_state,after_state,before_deployment_version,after_deployment_version,before_lifecycle,after_lifecycle,ready_retention_fact_id,event_id,event_cursor,event_aggregate_version)
        VALUES($1,$2,$3,$4,$5,$6,$7,'creation_retained_ready',$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20)")
        .bind(id(request)).bind(request.command.id().as_uuid()).bind(request.target.provenance.attempt_id.as_uuid())
        .bind(request.operation_id.as_uuid()).bind(request.target.deployment_id.as_uuid()).bind(context.snapshot.intent.project_id().as_uuid())
        .bind(request.target.resource.as_str()).bind(fingerprint(request)?.as_bytes().as_slice())
        .bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid()).bind(sqlx::types::Json(before)).bind(sqlx::types::Json(&after))
        .bind(i64::try_from(context.snapshot.version).map_err(|_|DeploymentError::StaleClaim)?)
        .bind(i64::try_from(after_version).map_err(|_|DeploymentError::StaleClaim)?)
        .bind(context.snapshot.lifecycle.as_str()).bind(lifecycle.as_str()).bind(fact).bind(event).bind(cursor).bind(version)
        .execute(&mut *context.tx).await.map_err(repository_error)?;
    let resource=sqlx::query("UPDATE recipe_deployment_resources SET install_progress='ready',removal_progress='retained',active_attempt_id=NULL,diagnostic=NULL,last_verified_action='retain',version=$3 WHERE deployment_id=$1 AND resource_name=$2 AND version=$4")
        .bind(request.target.deployment_id.as_uuid()).bind(request.target.resource.as_str()).bind(after.version).bind(before.version)
        .execute(&mut *context.tx).await.map_err(repository_error)?;
    let deployment = sqlx::query(
        "UPDATE recipe_deployments SET lifecycle=$2,version=$3 WHERE id=$1 AND version=$4",
    )
    .bind(request.target.deployment_id.as_uuid())
    .bind(lifecycle.as_str())
    .bind(i64::try_from(after_version).map_err(|_| DeploymentError::StaleClaim)?)
    .bind(i64::try_from(context.snapshot.version).map_err(|_| DeploymentError::StaleClaim)?)
    .execute(&mut *context.tx)
    .await
    .map_err(repository_error)?;
    if resource.rows_affected() != 1 || deployment.rows_affected() != 1 {
        return Err(DeploymentError::StaleClaim);
    }
    Ok(after)
}
