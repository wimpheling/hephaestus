use super::{PgForgeRepository, helpers::storage, rows::RepositoryRow};
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{audit_decision, begin_actor_transaction};
use forge_domain::{Repository, RepositoryId};
use forge_service::ForgeRepositoryError;
use identity_domain::AuthenticatedIdentity;

impl PgForgeRepository {
    /// Loads repository metadata and verifies its bare storage.
    ///
    /// # Errors
    ///
    /// Returns an error when metadata or canonical storage is absent.
    pub async fn get_repository(
        &self,
        id: RepositoryId,
    ) -> Result<Repository, ForgeRepositoryError> {
        let row = sqlx::query_as::<_, RepositoryRow>(
            "SELECT id, project_id, name, default_branch, is_public, settings, created_at
             FROM repositories WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(ForgeRepositoryError::RepositoryNotFound(id))?;
        self.storage.validate_existing(id).await?;
        row.try_into()
    }

    /// Loads repository metadata in a transaction carrying authenticated actor
    /// context.
    ///
    /// # Errors
    ///
    /// Returns an error when metadata or canonical storage is absent.
    pub async fn get_repository_as(
        &self,
        id: RepositoryId,
        identity: &AuthenticatedIdentity,
    ) -> Result<Repository, ForgeRepositoryError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(storage)?;
        let row = sqlx::query_as::<_, RepositoryRow>(
            "SELECT id, project_id, name, default_branch, is_public, settings, created_at
             FROM repositories WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?
        .ok_or(ForgeRepositoryError::RepositoryNotFound(id))?;
        transaction.commit().await.map_err(storage)?;
        self.storage.validate_existing(id).await?;
        row.try_into()
    }

    /// Deletes repository metadata and bare storage after an explicit
    /// `repository.can_delete` check.
    ///
    /// # Errors
    ///
    /// Returns an error for denial or a database/filesystem failure.
    pub async fn delete_repository(
        &self,
        identity: &AuthenticatedIdentity,
        id: RepositoryId,
    ) -> Result<(), ForgeRepositoryError> {
        let authorizer = self
            .authorizer
            .as_ref()
            .ok_or(ForgeRepositoryError::AuthorizationUnavailable)?;
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(storage)?;
        let object = ObjectRef::new(ObjectType::Repository, id.as_uuid());
        let decision = authorizer
            .check(
                &mut transaction,
                Subject::User(identity.user_id),
                Permission::CanDelete,
                object,
            )
            .await
            .map_err(storage)?;
        audit_decision(
            &mut transaction,
            identity.user_id,
            Permission::CanDelete,
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
        let deleted = sqlx::query("DELETE FROM repositories WHERE id = $1")
            .bind(id.as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        if deleted.rows_affected() == 0 {
            return Err(ForgeRepositoryError::RepositoryNotFound(id));
        }
        transaction.commit().await.map_err(storage)?;
        self.storage.delete_bare(id).await?;
        Ok(())
    }
}
