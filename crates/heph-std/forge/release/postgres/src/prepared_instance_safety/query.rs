use recipe_application::{CommandIdentity, DeploymentError, PreparedInstanceSafetyTarget};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};

use super::codec::Row;

pub async fn role(
    tx: &mut Transaction<'_, Postgres>,
    expected: &str,
) -> Result<(), DeploymentError> {
    let role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&mut **tx)
        .await
        .map_err(|error| storage(&error))?;
    if role != expected {
        return Err(DeploymentError::AuthorizationDenied);
    }
    Ok(())
}
pub async fn actor(
    tx: &mut Transaction<'_, Postgres>,
    command: CommandIdentity,
    target: &PreparedInstanceSafetyTarget,
) -> Result<Row, DeploymentError> {
    sqlx::query_as("SELECT birth_proven,removal_id,cleanup_ready,inventory FROM inspect_prepared_instance_safety118($1,$2,$3,$4,$5,$6)")
        .bind(command.id().as_uuid()).bind(target.original().provenance.attempt_id.as_uuid())
        .bind(claim(target)?).bind(serde_json::to_value(target.prepared()).map_err(|_| DeploymentError::Serialization)?)
        .bind(target.provider_namespace()).bind(target.host_id())
        .fetch_one(&mut **tx).await.map_err(|error| storage(&error))
}
pub async fn worker(
    tx: &mut Transaction<'_, Postgres>,
    target: &PreparedInstanceSafetyTarget,
) -> Result<Row, DeploymentError> {
    sqlx::query_as("SELECT birth_proven,removal_id,cleanup_ready,inventory FROM observe_prepared_instance_safety118($1,$2,$3,$4,$5)")
        .bind(target.original().provenance.attempt_id.as_uuid()).bind(claim(target)?)
        .bind(serde_json::to_value(target.prepared()).map_err(|_| DeploymentError::Serialization)?)
        .bind(target.provider_namespace()).bind(target.host_id())
        .fetch_one(&mut **tx).await.map_err(|error| storage(&error))
}
fn claim(target: &PreparedInstanceSafetyTarget) -> Result<Value, DeploymentError> {
    let original = target.original();
    Ok(
        json!({"command_id":original.command.id(),"deployment_id":original.deployment_id,"resource":original.resource,
        "identity":serde_json::to_value(original.identity).map_err(|_| DeploymentError::Serialization)?,
        "input_hash":original.input_hash,"resource_version":original.resource_version,"generation":original.generation,
        "action":original.action,"attempt_id":original.provenance.attempt_id,"actor_id":original.provenance.actor_id,
        "request_id":original.provenance.request_id}),
    )
}
pub fn storage(error: &sqlx::Error) -> DeploymentError {
    match error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .as_deref()
    {
        Some("42501") => DeploymentError::AuthorizationDenied,
        Some("P1180" | "P1420") => DeploymentError::ReconciliationRequired,
        _ => DeploymentError::Unavailable,
    }
}
