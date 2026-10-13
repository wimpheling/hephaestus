use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, DeploymentIntent, PlannedResourceIdentity, ResourceOwnership,
};
use recipe_domain::{ResolvedResource, ResourceDeclaration, ValidatedRecipe};
use sqlx::{Postgres, Transaction};

use crate::{PostgresDeploymentRepository, repository_error};

impl PostgresDeploymentRepository {
    pub(crate) async fn require(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        permission: Permission,
        object: ObjectRef,
    ) -> Result<(), DeploymentError> {
        let decision = PostgresMelangeAuthorizer
            .check(tx, Subject::User(identity.user_id), permission, object)
            .await
            .map_err(repository_error)?;
        audit_decision(
            tx,
            identity.user_id,
            permission,
            object,
            decision,
            identity.request_id,
        )
        .await
        .map_err(repository_error)?;
        if decision == AuthorizationDecision::Allow {
            return Ok(());
        }
        // The rejected command rolls back; its denial remains durable separately.
        let mut audit = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(repository_error)?;
        audit_decision(
            &mut audit,
            identity.user_id,
            permission,
            object,
            decision,
            identity.request_id,
        )
        .await
        .map_err(repository_error)?;
        audit.commit().await.map_err(repository_error)?;
        Err(DeploymentError::AuthorizationDenied)
    }

    pub(crate) async fn install_authority(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        declaration: &ValidatedRecipe,
        intent: &DeploymentIntent,
    ) -> Result<(), DeploymentError> {
        self.require(
            tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, intent.project_id().as_uuid()),
        )
        .await?;
        for resource in &declaration.manifest().resources {
            if let ResourceDeclaration::Instance(instance) = resource {
                self.require(
                    tx,
                    identity,
                    Permission::CanUse,
                    ObjectRef::new(
                        ObjectType::ReleaseAgent,
                        instance.release.release_agent_id.as_uuid(),
                    ),
                )
                .await?;
            }
        }
        for resource in intent.resolved().resources().values() {
            if let ResolvedResource::Volume(volume) = resource
                && let Some(id) = volume.external_id
            {
                let object = ObjectRef::new(ObjectType::StateVolume, id.as_uuid());
                self.require(tx, identity, Permission::CanRead, object)
                    .await?;
                let selected = declaration.manifest().resources.iter().any(|declaration| {
                    if let ResourceDeclaration::Instance(instance) = declaration {
                        instance.volume_bindings.iter().any(|binding| {
                            matches!(intent.resolved().resources().get(&binding.resource),
                                Some(ResolvedResource::Volume(bound)) if bound.external_id == Some(id))
                        })
                    } else { false }
                });
                if !selected {
                    continue;
                }
                for permission in [Permission::CanAttach, Permission::CanGrantAgentCapability] {
                    self.require(tx, identity, permission, object).await?;
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn cleanup_authority(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        intent: &DeploymentIntent,
    ) -> Result<(), DeploymentError> {
        for resource in intent.resources().values() {
            if resource.ownership() != ResourceOwnership::Owned {
                continue;
            }
            let object = match resource.identity() {
                PlannedResourceIdentity::Volume { id, .. } => {
                    ObjectRef::new(ObjectType::StateVolume, id.as_uuid())
                }
                PlannedResourceIdentity::Instance { id, .. } => {
                    ObjectRef::new(ObjectType::AgentInstance, id.as_uuid())
                }
            };
            let project: Option<uuid::Uuid> =
                sqlx::query_scalar("SELECT recipe_owned_resource_project($1, $2)")
                    .bind(object.object_type.as_str())
                    .bind(object.id)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(repository_error)?;
            if let Some(project) = project {
                if project != intent.project_id().as_uuid() {
                    return Err(DeploymentError::IntentMismatch);
                }
                self.require(tx, identity, Permission::CanManage, object)
                    .await?;
            }
        }
        Ok(())
    }
}
