use crate::{
    PostgresDeploymentRepository, catalog, effect_state::State, hydration, persistence, receipts,
    repository_error,
};
use authz_domain::{ObjectRef, ObjectType, Permission};
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentId, DeploymentOperation, DeploymentSnapshot,
};
use sqlx::{Postgres, Transaction};

pub struct Context<'a> {
    pub tx: Transaction<'a, Postgres>,
    pub snapshot: DeploymentSnapshot,
    pub organization: uuid::Uuid,
    pub cleanup: bool,
    pub command: CommandIdentity,
}

impl PostgresDeploymentRepository {
    pub(crate) async fn execution_context<'a>(
        &'a self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        id: DeploymentId,
        allow_cleanup_observation: bool,
    ) -> Result<Context<'a>, DeploymentError> {
        command.validate(identity, command.operation())?;
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(repository_error)?;
        let project = persistence::project(&mut tx, id).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, project.as_uuid()),
        )
        .await?;
        let organization = persistence::lock_project(&mut tx, project).await?;
        let snapshot = hydration::load(&mut tx, id).await?;
        let cleanup: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recipe_deployment_commands WHERE deployment_id = $1 AND operation = 'remove')")
            .bind(id.as_uuid()).fetch_one(&mut *tx).await.map_err(repository_error)?;
        if command.operation() == DeploymentOperation::Install
            && (!cleanup || !allow_cleanup_observation)
        {
            let declaration =
                recipe_domain::parse_recipe(snapshot.intent.declaration_toml().as_bytes())?;
            self.install_authority(&mut tx, identity, &declaration, &snapshot.intent)
                .await?;
            let evidence = catalog::load(&mut tx, &declaration, &snapshot.intent).await?;
            catalog::reconstruct(&snapshot.intent, &declaration, &evidence)?;
        }
        self.cleanup_authority(&mut tx, identity, &snapshot.intent)
            .await?;
        let row = receipts::find(&mut tx, command)
            .await?
            .ok_or(DeploymentError::Unavailable)?;
        if row.deployment_id != id.as_uuid() || row.id != command.id().as_uuid() {
            return Err(DeploymentError::InputConflict);
        }
        Ok(Context {
            tx,
            snapshot,
            organization,
            cleanup,
            command,
        })
    }
}

pub async fn state(
    tx: &mut Transaction<'_, Postgres>,
    id: DeploymentId,
    resource: &capability_domain::CapabilitySlotKey,
) -> Result<State, DeploymentError> {
    let value: sqlx::types::Json<State> = sqlx::query_scalar(
        "SELECT recipe_effect_resource_state(resource) FROM recipe_deployment_resources resource
         WHERE deployment_id = $1 AND resource_name = $2 FOR UPDATE",
    )
    .bind(id.as_uuid())
    .bind(resource.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(repository_error)?
    .ok_or(DeploymentError::Unavailable)?;
    Ok(value.0)
}
