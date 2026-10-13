//! The writer's same permission evaluator, without audits or locks in RRRO.
use crate::repository_error;
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::PostgresMelangeAuthorizer;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, DeploymentIntent, PlannedResourceIdentity, ResourceOwnership,
};
use sqlx::{Postgres, Transaction};

pub(super) async fn actor(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
) -> Result<(), DeploymentError> {
    if identity.user_id.as_uuid().is_nil()
        || identity.request_id.as_uuid().is_nil()
        || identity.idempotency_id.as_uuid().is_nil()
    {
        return Err(DeploymentError::InvalidIdentifier);
    }
    sqlx::query(
        "SELECT set_config('search_path','pg_catalog,public,pg_temp',true),
        set_config('hephaestus.subject_type','user',true),set_config('hephaestus.actor_id',$1,true),
        set_config('hephaestus.request_id',$2,true),set_config('hephaestus.occurrence_id',$3,true)",
    )
    .bind(identity.user_id.to_string())
    .bind(identity.request_id.to_string())
    .bind(identity.idempotency_id.to_string())
    .execute(&mut **tx)
    .await
    .map_err(repository_error)?;
    let active: bool = sqlx::query_scalar("SELECT current_user::text='hephaestus_app' AND EXISTS(SELECT 1 FROM users WHERE id=$1 AND status='active')")
        .bind(identity.user_id.as_uuid()).fetch_one(&mut **tx).await.map_err(repository_error)?;
    if !active {
        return Err(DeploymentError::AuthorizationDenied);
    }
    Ok(())
}

pub(super) async fn project(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    intent: &DeploymentIntent,
) -> Result<(), DeploymentError> {
    require(
        tx,
        identity,
        ObjectRef::new(ObjectType::Project, intent.project_id().as_uuid()),
    )
    .await
}

pub(super) async fn owned(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    intent: &DeploymentIntent,
) -> Result<(), DeploymentError> {
    for plan in intent.resources().values() {
        if plan.ownership() != ResourceOwnership::Owned {
            continue;
        }
        let object = match plan.identity() {
            PlannedResourceIdentity::Instance { id, .. } => {
                ObjectRef::new(ObjectType::AgentInstance, id.as_uuid())
            }
            PlannedResourceIdentity::Volume { id, .. } => {
                ObjectRef::new(ObjectType::StateVolume, id.as_uuid())
            }
        };
        let project: Option<uuid::Uuid> =
            sqlx::query_scalar("SELECT recipe_owned_resource_project($1,$2)")
                .bind(object.object_type.as_str())
                .bind(object.id)
                .fetch_one(&mut **tx)
                .await
                .map_err(repository_error)?;
        if let Some(project) = project {
            if project != intent.project_id().as_uuid() {
                return Err(DeploymentError::IntentMismatch);
            }
            require(tx, identity, object).await?;
        }
    }
    Ok(())
}

async fn require(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    object: ObjectRef,
) -> Result<(), DeploymentError> {
    let decision = PostgresMelangeAuthorizer
        .check(
            tx,
            Subject::User(identity.user_id),
            Permission::CanManage,
            object,
        )
        .await
        .map_err(repository_error)?;
    if decision == AuthorizationDecision::Allow {
        Ok(())
    } else {
        Err(DeploymentError::AuthorizationDenied)
    }
}
