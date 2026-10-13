//! Current-manager comparison of an actual completed original Volume birth.
mod birth;
mod current;
mod fact_identity;
pub(super) use fact_identity::load as retained_fact;
mod ledger;
mod selected;

use crate::{PostgresDeploymentRepository, repository_error};
use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentIntent, DeploymentOperation, EffectClaim,
    PlannedResourceIdentity, PreparedOwnedVolumeCreation, ResourceOwnership,
};
use sqlx::{Postgres, Transaction};

pub(super) async fn load(
    repository: &PostgresDeploymentRepository,
    identity: &AuthenticatedIdentity,
    command: CommandIdentity,
    intent: &DeploymentIntent,
    resource: &CapabilitySlotKey,
) -> Result<(EffectClaim, PreparedOwnedVolumeCreation), DeploymentError> {
    command.validate(identity, DeploymentOperation::Remove)?;
    let config = repository
        .remove_resume
        .as_ref()
        .ok_or(DeploymentError::InvalidAction)?;
    let plan = intent
        .resources()
        .get(resource)
        .ok_or(DeploymentError::IntentMismatch)?;
    if intent.project_id().as_uuid().is_nil()
        || plan.ownership() != ResourceOwnership::Owned
        || !matches!(
            plan.identity(),
            PlannedResourceIdentity::Volume {
                filesystem_uuid: Some(_),
                ..
            }
        )
    {
        return Err(DeploymentError::InvalidAction);
    }
    let mut tx = repository
        .pool
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .map_err(repository_error)?;
    let result = selected::read(&mut tx, identity, command, intent, resource, config).await;
    let rollback = tx.rollback().await.map_err(repository_error);
    match result {
        Err(primary) => Err(primary),
        Ok(value) => rollback.map(|()| value),
    }
}

async fn actor(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
) -> Result<(), DeploymentError> {
    let role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&mut **tx)
        .await
        .map_err(repository_error)?;
    if role != "hephaestus_app" {
        return Err(DeploymentError::AuthorizationDenied);
    }
    sqlx::query(
        "SELECT set_config('search_path','pg_catalog,public,pg_temp',true),
        set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.subject_type','user',true),
        set_config('hephaestus.request_id',$2,true),set_config('hephaestus.occurrence_id',$3,true)",
    )
    .bind(identity.user_id.to_string())
    .bind(identity.request_id.to_string())
    .bind(identity.idempotency_id.to_string())
    .execute(&mut **tx)
    .await
    .map_err(repository_error)?;
    Ok(())
}
