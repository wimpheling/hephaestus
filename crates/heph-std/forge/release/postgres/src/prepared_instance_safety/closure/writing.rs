use super::super::query;
use crate::{ReleaseServiceError, release_instance_removal_work};
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, DeploymentExecutionPreparation, DeploymentIntent, EffectClaim,
    PreparedInstanceClosureRequest,
};
use release_service::RequestInstanceRemoval;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};

const fn command(request: &PreparedInstanceClosureRequest) -> RequestInstanceRemoval {
    RequestInstanceRemoval {
        command_key: request.command_key(),
        removal_id: request.removal_id(),
        instance_id: request.target().prepared().instance_id(),
        expected_version: request.expected_instance_version(),
    }
}
pub fn claim(claim: &EffectClaim) -> Result<Value, DeploymentError> {
    Ok(
        json!({"command_id":claim.command.id(),"deployment_id":claim.deployment_id,"resource":claim.resource,
        "identity":serde_json::to_value(claim.identity).map_err(|_|DeploymentError::Serialization)?,
        "input_hash":claim.input_hash,"resource_version":claim.resource_version,"generation":claim.generation,
        "action":claim.action,"attempt_id":claim.provenance.attempt_id,"actor_id":claim.provenance.actor_id,"request_id":claim.provenance.request_id}),
    )
}
pub async fn qualify(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    request: &PreparedInstanceClosureRequest,
    whole: &DeploymentExecutionPreparation,
) -> Result<bool, DeploymentError> {
    let hash = release_instance_removal_work::input_hash(identity, &command(request))
        .map_err(|error| storage(&error))?;
    sqlx::query_scalar("SELECT prepare_prepared_instance_closure118($1,$2,$3,$4,$5,$6,$7,$8,$9)")
        .bind(request.target().original().provenance.attempt_id.as_uuid())
        .bind(claim(request.target().original())?)
        .bind(
            serde_json::to_value(request.target().prepared())
                .map_err(|_| DeploymentError::Serialization)?,
        )
        .bind(whole.canonical_bytes()?)
        .bind(claim(request.drain())?)
        .bind(request.command_key().as_bytes().as_slice())
        .bind(request.removal_id().as_uuid())
        .bind(
            i64::try_from(request.expected_instance_version())
                .map_err(|_| DeploymentError::IntentMismatch)?,
        )
        .bind(hash.as_slice())
        .fetch_one(&mut **tx)
        .await
        .map_err(|error| query::storage(&error))
}
pub async fn write(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    request: &PreparedInstanceClosureRequest,
) -> Result<(), DeploymentError> {
    audit_all(tx, identity, request.intent()).await?;
    let command = command(request);
    let hash = release_instance_removal_work::input_hash(identity, &command)
        .map_err(|error| storage(&error))?;
    release_instance_removal_work::admit_request(tx, identity, &command, &hash)
        .await
        .map_err(|error| storage(&error))
}
pub async fn audit_all(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    intent: &DeploymentIntent,
) -> Result<(), DeploymentError> {
    // SQL already repeated ALL-owned permissions under parents/late participant
    // locks. These genuine checks persist the current Remove decision audits.
    audit(
        tx,
        identity,
        Permission::CanManage,
        ObjectRef::new(ObjectType::Project, intent.project_id().as_uuid()),
    )
    .await?;
    let objects: Vec<(String,uuid::Uuid)> = sqlx::query_as("SELECT resource_kind,resource_id FROM recipe_deployment_resources WHERE deployment_id=$1 AND ownership='owned' ORDER BY resource_name")
        .bind(intent.id().as_uuid()).fetch_all(&mut **tx).await.map_err(|error|query::storage(&error))?;
    for (kind, id) in objects {
        let object = match kind.as_str() {
            "instance" => ObjectType::AgentInstance,
            "volume" => ObjectType::StateVolume,
            _ => return Err(DeploymentError::ReconciliationRequired),
        };
        // Positive object existence is required for these audit decisions. A
        // not-yet-created owned object is handled by the SQL parent/cleanup check.
        let exists: bool =
            sqlx::query_scalar("SELECT recipe_owned_resource_project($1,$2) IS NOT NULL")
                .bind(object.as_str())
                .bind(id)
                .fetch_one(&mut **tx)
                .await
                .map_err(|error| query::storage(&error))?;
        if exists {
            audit(
                tx,
                identity,
                Permission::CanManage,
                ObjectRef::new(object, id),
            )
            .await?;
        }
    }
    Ok(())
}
pub(super) const fn storage(error: &ReleaseServiceError) -> DeploymentError {
    match error {
        ReleaseServiceError::AuthorizationDenied => DeploymentError::AuthorizationDenied,
        _ => DeploymentError::Unavailable,
    }
}

async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    permission: Permission,
    object: ObjectRef,
) -> Result<(), DeploymentError> {
    let decision = PostgresMelangeAuthorizer
        .check(tx, Subject::User(identity.user_id), permission, object)
        .await
        .map_err(|_| DeploymentError::Unavailable)?;
    if decision != AuthorizationDecision::Allow {
        return Err(DeploymentError::AuthorizationDenied);
    }
    // No independent denial transaction while holding parent/participant locks.
    audit_decision(
        tx,
        identity.user_id,
        permission,
        object,
        decision,
        identity.request_id,
    )
    .await
    .map_err(|error| query::storage(&error))?;
    Ok(())
}
