use super::{
    PgForgeRepository,
    helpers::storage,
    rows::ProjectRow,
    validation::{insert_repository, validate_description, validate_name, validate_repository},
};
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{audit_decision, begin_actor_transaction};
use forge_domain::{OrganizationId, Project, ProjectId, Repository, RepositoryId};
use forge_service::{CreateRepository, ForgeRepositoryError};
use identity_domain::AuthenticatedIdentity;
use serde_json::json;

impl PgForgeRepository {
    /// Applies all workspace migrations.
    ///
    /// # Errors
    ///
    /// Returns an error when migration application fails.
    pub async fn initialize(&self) -> Result<(), ForgeRepositoryError> {
        sqlx::migrate!("../../../../migrations")
            .run(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }

    /// Creates one durable project after checking the owning organization.
    ///
    /// # Errors
    ///
    /// Returns an error for denial, an invalid name, or persistence failure.
    pub async fn create_project(
        &self,
        identity: &AuthenticatedIdentity,
        organization_id: OrganizationId,
        name: &str,
    ) -> Result<Project, ForgeRepositoryError> {
        self.create_project_with_description(identity, organization_id, name, "")
            .await
    }

    /// Creates a durable project with an optional human-readable description
    /// after checking the owning organization.
    ///
    /// The description is stored in the existing project settings JSON object
    /// so this additive metadata does not require a schema migration.
    ///
    /// # Errors
    ///
    /// Returns an error for denial, invalid metadata, or persistence failure.
    pub async fn create_project_with_description(
        &self,
        identity: &AuthenticatedIdentity,
        organization_id: OrganizationId,
        name: &str,
        description: &str,
    ) -> Result<Project, ForgeRepositoryError> {
        validate_name(name, "project name must contain 1 to 200 characters")?;
        validate_description(description)?;
        let authorizer = self
            .authorizer
            .as_ref()
            .ok_or(ForgeRepositoryError::AuthorizationUnavailable)?;
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(storage)?;
        let object = ObjectRef::new(ObjectType::Organization, organization_id.as_uuid());
        let decision = authorizer
            .check(
                &mut transaction,
                Subject::User(identity.user_id),
                Permission::CanCreateProject,
                object,
            )
            .await
            .map_err(storage)?;
        audit_decision(
            &mut transaction,
            identity.user_id,
            Permission::CanCreateProject,
            object,
            decision,
            identity.request_id,
        )
        .await
        .map_err(storage)?;
        if decision == AuthorizationDecision::Deny {
            transaction.commit().await.map_err(storage)?;
            return Err(ForgeRepositoryError::AuthorizationDenied);
        }
        let id = ProjectId::new();
        let row = sqlx::query_as::<_, ProjectRow>(
            "INSERT INTO projects (id, organization_id, name, settings)
             VALUES ($1, $2, $3, $4)
             RETURNING id, organization_id, name, created_at",
        )
        .bind(id.as_uuid())
        .bind(organization_id.as_uuid())
        .bind(name)
        .bind(json!({"description": description}))
        .fetch_one(&mut *transaction)
        .await
        .map_err(storage)?;
        sqlx::query("SELECT ensure_project_maintainer($1, $2)")
            .bind(row.id)
            .bind(identity.user_id.as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        Ok(row.into())
    }

    /// Creates a project from trusted bootstrap or test code.
    ///
    /// Request-facing code must call [`Self::create_project`].
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid name or persistence failure.
    pub async fn create_project_trusted(
        &self,
        organization_id: OrganizationId,
        name: &str,
    ) -> Result<Project, ForgeRepositoryError> {
        self.create_project_trusted_with_description(organization_id, name, "")
            .await
    }

    /// Creates a project with a description from trusted bootstrap or test
    /// code.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid metadata or persistence failure.
    pub async fn create_project_trusted_with_description(
        &self,
        organization_id: OrganizationId,
        name: &str,
        description: &str,
    ) -> Result<Project, ForgeRepositoryError> {
        validate_name(name, "project name must contain 1 to 200 characters")?;
        validate_description(description)?;
        let id = ProjectId::new();
        let row = sqlx::query_as::<_, ProjectRow>(
            "INSERT INTO projects (id, organization_id, name, settings)
             VALUES ($1, $2, $3, $4)
             RETURNING id, organization_id, name, created_at",
        )
        .bind(id.as_uuid())
        .bind(organization_id.as_uuid())
        .bind(name)
        .bind(json!({"description": description}))
        .fetch_one(&self.pool)
        .await
        .map_err(storage)?;
        Ok(row.into())
    }

    /// Creates metadata after checking the parent project, then initializes its
    /// canonical bare repository.
    ///
    /// A failed Git initialization compensates by removing the uncommitted
    /// metadata row. No caller-supplied path enters the storage operation.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid metadata, storage, or database failures.
    pub async fn create_repository(
        &self,
        identity: &AuthenticatedIdentity,
        input: &CreateRepository,
    ) -> Result<Repository, ForgeRepositoryError> {
        let branch = validate_repository(input)?;
        let authorizer = self
            .authorizer
            .as_ref()
            .ok_or(ForgeRepositoryError::AuthorizationUnavailable)?;
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(storage)?;
        let object = ObjectRef::new(ObjectType::Project, input.project_id.as_uuid());
        let decision = authorizer
            .check(
                &mut transaction,
                Subject::User(identity.user_id),
                Permission::CanWrite,
                object,
            )
            .await
            .map_err(storage)?;
        audit_decision(
            &mut transaction,
            identity.user_id,
            Permission::CanWrite,
            object,
            decision,
            identity.request_id,
        )
        .await
        .map_err(storage)?;
        if decision == AuthorizationDecision::Deny {
            transaction.commit().await.map_err(storage)?;
            return Err(ForgeRepositoryError::AuthorizationDenied);
        }
        let id = RepositoryId::new();
        let row = insert_repository(&mut transaction, id, input).await?;
        transaction.commit().await.map_err(storage)?;
        if let Err(error) = self.storage.create_bare(id, branch).await {
            sqlx::query("DELETE FROM repositories WHERE id = $1")
                .bind(id.as_uuid())
                .execute(&self.pool)
                .await
                .map_err(storage)?;
            return Err(error.into());
        }
        row.try_into()
    }

    /// Creates a repository from trusted bootstrap or test code.
    ///
    /// Request-facing code must call [`Self::create_repository`].
    ///
    /// # Errors
    ///
    /// Returns an error for invalid metadata, storage, or persistence failure.
    pub async fn create_repository_trusted(
        &self,
        input: &CreateRepository,
    ) -> Result<Repository, ForgeRepositoryError> {
        let branch = validate_repository(input)?;
        let id = RepositoryId::new();
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let row = insert_repository(&mut transaction, id, input).await?;
        transaction.commit().await.map_err(storage)?;
        if let Err(error) = self.storage.create_bare(id, branch).await {
            sqlx::query("DELETE FROM repositories WHERE id = $1")
                .bind(id.as_uuid())
                .execute(&self.pool)
                .await
                .map_err(storage)?;
            return Err(error.into());
        }
        row.try_into()
    }
}
