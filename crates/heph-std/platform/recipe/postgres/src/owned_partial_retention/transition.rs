use crate::{
    effect_context::Context, effect_state::State, execution_transitions, repository_error,
};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, DeploymentLifecycle, RemovalProgress, RetainPartialCreation,
};
use release_domain::ContentHash;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub fn id(request: &RetainPartialCreation) -> Uuid {
    execution_transitions::transition_id(
        request.operation_id.as_uuid(),
        "creation_retained_partial",
    )
}
pub fn fingerprint(request: &RetainPartialCreation) -> Result<ContentHash, DeploymentError> {
    execution_transitions::fingerprint(
        &request.target,
        "creation_retained_partial",
        &serde_json::json!([
            request.command,
            request.operation_id,
            request.expected_deployment_version,
            request.expected_resource_version
        ]),
    )
}
pub async fn exact_replay_receipt(
    tx: &mut Transaction<'_, Postgres>,
    request: &RetainPartialCreation,
    receipt: Uuid,
) -> Result<(), DeploymentError> {
    let stored: Option<Uuid> = sqlx::query_scalar(
        "SELECT partial_retention_receipt_id FROM recipe_execution_transitions WHERE id=$1",
    )
    .bind(id(request))
    .fetch_one(&mut **tx)
    .await
    .map_err(repository_error)?;
    if stored == Some(receipt) {
        Ok(())
    } else {
        Err(DeploymentError::InputConflict)
    }
}
pub async fn commit(
    context: &mut Context<'_>,
    identity: &AuthenticatedIdentity,
    request: &RetainPartialCreation,
    before: &State,
    receipt: Uuid,
) -> Result<State, DeploymentError> {
    let mut after = before.clone();
    after.active_attempt = None;
    after.removal = RemovalProgress::Retained;
    after.version = before
        .version
        .checked_add(1)
        .ok_or(DeploymentError::StaleClaim)?;
    let after_version = context
        .snapshot
        .version
        .checked_add(1)
        .ok_or(DeploymentError::StaleClaim)?;
    let other_uncertain:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recipe_deployment_resources WHERE deployment_id=$1 AND resource_name<>$2 AND active_attempt_id IS NOT NULL AND (install_progress IN ('failed','recovery_required') OR removal_progress='recovery_required'))")
        .bind(request.target.deployment_id.as_uuid()).bind(request.target.resource.as_str()).fetch_one(&mut *context.tx).await.map_err(repository_error)?;
    let lifecycle = if other_uncertain {
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
    sqlx::query("INSERT INTO recipe_execution_transitions(id,command_id,attempt_id,operation_id,deployment_id,project_id,resource_name,kind,fingerprint,actor_id,request_id,before_state,after_state,before_deployment_version,after_deployment_version,before_lifecycle,after_lifecycle,partial_retention_receipt_id,event_id,event_cursor,event_aggregate_version) VALUES($1,$2,$3,$4,$5,$6,$7,'creation_retained_partial',$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20)")
        .bind(id(request)).bind(request.command.id().as_uuid()).bind(request.target.provenance.attempt_id.as_uuid()).bind(request.operation_id.as_uuid())
        .bind(request.target.deployment_id.as_uuid()).bind(context.snapshot.intent.project_id().as_uuid()).bind(request.target.resource.as_str())
        .bind(fingerprint(request)?.as_bytes().as_slice()).bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid())
        .bind(sqlx::types::Json(before)).bind(sqlx::types::Json(&after)).bind(i64::try_from(context.snapshot.version).map_err(|_|DeploymentError::StaleClaim)?)
        .bind(i64::try_from(after_version).map_err(|_|DeploymentError::StaleClaim)?).bind(context.snapshot.lifecycle.as_str()).bind(lifecycle.as_str())
        .bind(receipt).bind(event).bind(cursor).bind(version).execute(&mut *context.tx).await.map_err(repository_error)?;
    let updated=sqlx::query("UPDATE recipe_deployment_resources SET removal_progress='retained',version=$3,active_attempt_id=NULL WHERE deployment_id=$1 AND resource_name=$2 AND version=$4")
        .bind(request.target.deployment_id.as_uuid()).bind(request.target.resource.as_str()).bind(after.version).bind(before.version)
        .execute(&mut *context.tx).await.map_err(repository_error)?;
    if updated.rows_affected() != 1 {
        return Err(DeploymentError::StaleClaim);
    }
    let updated = sqlx::query(
        "UPDATE recipe_deployments SET lifecycle=$2,version=$3 WHERE id=$1 AND version=$4",
    )
    .bind(request.target.deployment_id.as_uuid())
    .bind(lifecycle.as_str())
    .bind(i64::try_from(after_version).map_err(|_| DeploymentError::StaleClaim)?)
    .bind(i64::try_from(context.snapshot.version).map_err(|_| DeploymentError::StaleClaim)?)
    .execute(&mut *context.tx)
    .await
    .map_err(repository_error)?;
    if updated.rows_affected() != 1 {
        return Err(DeploymentError::StaleClaim);
    }
    Ok(after)
}
