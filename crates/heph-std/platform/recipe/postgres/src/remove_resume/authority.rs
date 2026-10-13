use crate::repository_error;
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, DeploymentIntent, PlannedResourceIdentity, ResourceOwnership,
};
use sqlx::{Postgres, Transaction};

pub async fn all(
    tx: &mut Transaction<'_, Postgres>,
    actor: &AuthenticatedIdentity,
    intent: &DeploymentIntent,
) -> Result<(), DeploymentError> {
    let active: bool = sqlx::query_scalar("SELECT current_user::text='hephaestus_app' AND EXISTS(SELECT 1 FROM users WHERE id=$1 AND status='active')")
        .bind(actor.user_id.as_uuid()).fetch_one(&mut **tx).await.map_err(repository_error)?;
    if !active {
        return Err(DeploymentError::AuthorizationDenied);
    }
    audit(
        tx,
        actor,
        ObjectRef::new(ObjectType::Project, intent.project_id().as_uuid()),
    )
    .await?;
    for resource in intent.resources().values() {
        if resource.ownership() != ResourceOwnership::Owned {
            continue;
        }
        let object = match resource.identity() {
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
            audit(tx, actor, object).await?;
        }
    }
    Ok(())
}
async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    actor: &AuthenticatedIdentity,
    object: ObjectRef,
) -> Result<(), DeploymentError> {
    let decision = PostgresMelangeAuthorizer
        .check(
            tx,
            Subject::User(actor.user_id),
            Permission::CanManage,
            object,
        )
        .await
        .map_err(repository_error)?;
    if decision != AuthorizationDecision::Allow {
        return Err(DeploymentError::AuthorizationDenied);
    }
    // Never start an independent denial transaction while holding project/event locks.
    audit_decision(
        tx,
        actor.user_id,
        Permission::CanManage,
        object,
        decision,
        actor.request_id,
    )
    .await
    .map_err(repository_error)?;
    Ok(())
}
