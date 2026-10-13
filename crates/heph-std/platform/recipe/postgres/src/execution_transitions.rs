use crate::{
    effect_context::Context, effect_state::State, execution_rows::TransitionRow, repository_error,
};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{DeploymentError, DeploymentLifecycle, EffectClaim};
use release_domain::ContentHash;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub fn transition_id(operation: Uuid, kind: &str) -> Uuid {
    Uuid::new_v5(&operation, kind.as_bytes())
}

pub fn fingerprint(
    claim: &EffectClaim,
    kind: &str,
    input: &serde_json::Value,
) -> Result<ContentHash, DeploymentError> {
    serde_json::to_vec(&(crate::execution_rows::ClaimWire::new(claim)?, kind, input))
        .map(|bytes| ContentHash::digest(&bytes))
        .map_err(|_| DeploymentError::Serialization)
}

pub async fn replay(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    fingerprint: ContentHash,
) -> Result<Option<State>, DeploymentError> {
    let row: Option<TransitionRow> = sqlx::query_as(
        "SELECT fingerprint, after_state FROM recipe_execution_transitions WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(repository_error)?;
    row.map(|row| {
        if row.fingerprint.as_slice() != fingerprint.as_bytes() {
            return Err(DeploymentError::InputConflict);
        }
        row.after_state
            .map(|state| state.0)
            .ok_or(DeploymentError::IntentMismatch)
    })
    .transpose()
}

pub struct Change<'a> {
    pub id: Uuid,
    pub kind: &'a str,
    pub operation_id: Uuid,
    pub fingerprint: ContentHash,
    pub claim: &'a EffectClaim,
    pub before: &'a State,
    pub after: &'a State,
    pub proof: Option<Uuid>,
}

pub async fn commit(
    context: &mut Context<'_>,
    identity: &AuthenticatedIdentity,
    change: Change<'_>,
) -> Result<State, DeploymentError> {
    let after_version = context
        .snapshot
        .version
        .checked_add(1)
        .ok_or(DeploymentError::StaleClaim)?;
    let other_uncertain: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recipe_deployment_resources
        WHERE deployment_id = $1 AND resource_name <> $2 AND active_attempt_id IS NOT NULL
          AND (install_progress IN ('failed','recovery_required') OR removal_progress = 'recovery_required'))")
        .bind(change.claim.deployment_id.as_uuid()).bind(change.claim.resource.as_str())
        .fetch_one(&mut *context.tx).await.map_err(repository_error)?;
    let uncertain = change.after.active_attempt.is_some()
        && (matches!(
            change.after.install,
            recipe_application::InstallProgress::Failed
                | recipe_application::InstallProgress::RecoveryRequired
        ) || change.after.removal == recipe_application::RemovalProgress::RecoveryRequired);
    let lifecycle = if uncertain || other_uncertain {
        DeploymentLifecycle::RecoveryRequired
    } else if context.snapshot.lifecycle == DeploymentLifecycle::RecoveryRequired {
        if context.cleanup {
            DeploymentLifecycle::Removing
        } else {
            DeploymentLifecycle::Installing
        }
    } else {
        context.snapshot.lifecycle
    };
    let (event_id, cursor, aggregate_version) = event(
        &mut context.tx,
        identity,
        change.id,
        context.organization,
        context.snapshot.intent.project_id(),
    )
    .await?;
    sqlx::query("INSERT INTO recipe_execution_transitions (id, command_id, attempt_id, operation_id, deployment_id, project_id,
        resource_name, kind, fingerprint, actor_id, request_id, before_state, after_state, before_deployment_version,
        after_deployment_version, before_lifecycle, after_lifecycle, verification_id, event_id, event_cursor, event_aggregate_version)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)")
        .bind(change.id).bind(context.command.id().as_uuid()).bind(change.claim.provenance.attempt_id.as_uuid())
        .bind(change.operation_id).bind(change.claim.deployment_id.as_uuid()).bind(context.snapshot.intent.project_id().as_uuid())
        .bind(change.claim.resource.as_str()).bind(change.kind).bind(change.fingerprint.as_bytes().as_slice())
        .bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid())
        .bind(sqlx::types::Json(change.before)).bind(sqlx::types::Json(change.after))
        .bind(i64::try_from(context.snapshot.version).map_err(|_| DeploymentError::StaleClaim)?)
        .bind(i64::try_from(after_version).map_err(|_| DeploymentError::StaleClaim)?)
        .bind(context.snapshot.lifecycle.as_str()).bind(lifecycle.as_str()).bind(change.proof)
        .bind(event_id).bind(cursor).bind(aggregate_version).execute(&mut *context.tx).await.map_err(repository_error)?;
    let resource_update = sqlx::query("UPDATE recipe_deployment_resources SET install_progress=$3, removal_progress=$4, version=$5,
        active_attempt_id=$6, diagnostic=$7, execution_generation=$8, last_verified_action=$9
        WHERE deployment_id=$1 AND resource_name=$2 AND version=$10")
        .bind(change.claim.deployment_id.as_uuid()).bind(change.claim.resource.as_str()).bind(change.after.install.as_str())
        .bind(change.after.removal.as_str()).bind(change.after.version).bind(change.after.active_attempt)
        .bind(change.after.diagnostic.map(recipe_application::DiagnosticCode::as_str)).bind(change.after.generation)
        .bind(change.after.last_action.map(recipe_application::ResourceAction::as_str)).bind(change.before.version)
        .execute(&mut *context.tx).await.map_err(repository_error)?;
    if resource_update.rows_affected() != 1 {
        return Err(DeploymentError::StaleClaim);
    }
    let deployment_update = sqlx::query(
        "UPDATE recipe_deployments SET lifecycle=$2, version=version+1 WHERE id=$1 AND version=$3",
    )
    .bind(change.claim.deployment_id.as_uuid())
    .bind(lifecycle.as_str())
    .bind(i64::try_from(context.snapshot.version).map_err(|_| DeploymentError::StaleClaim)?)
    .execute(&mut *context.tx)
    .await
    .map_err(repository_error)?;
    if deployment_update.rows_affected() != 1 {
        return Err(DeploymentError::StaleClaim);
    }
    context.snapshot.version = after_version;
    context.snapshot.lifecycle = lifecycle;
    Ok(change.after.clone())
}

pub async fn event(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    occurrence: Uuid,
    organization: Uuid,
    project: forge_domain::ProjectId,
) -> Result<(Uuid, i64, i64), DeploymentError> {
    // Each progress event has its own stable occurrence. Admission receipts stay exact.
    sqlx::query("SELECT set_config('hephaestus.occurrence_id', $1, true)")
        .bind(occurrence.to_string())
        .execute(&mut **tx)
        .await
        .map_err(repository_error)?;
    if identity.request_id.as_uuid().is_nil() {
        return Err(DeploymentError::InvalidIdentifier);
    }
    sqlx::query_as(
        "SELECT event_id, cursor, aggregate_version FROM append_application_event(
        $1, 'project', $2, 'project', $2, 'project.changed', 'updated', NULL, $3, NULL)",
    )
    .bind(occurrence)
    .bind(project.as_uuid())
    .bind(organization)
    .fetch_one(&mut **tx)
    .await
    .map_err(repository_error)
}
