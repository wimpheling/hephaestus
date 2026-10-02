use authz_domain::{ObjectRef, ObjectType, Permission};
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AdmissionDisposition, DeploymentAdmission, DeploymentError, DeploymentId, DeploymentLifecycle,
    DeploymentOperation, DeploymentSnapshot, InstallDeployment, RemoveDeployment,
};

use crate::{
    PostgresDeploymentRepository, catalog, hydration, persistence, receipts, repository_error,
};

impl PostgresDeploymentRepository {
    /// Admits immutable install intent after authoritative resolution and live checks.
    ///
    /// # Errors
    /// Rejects denied access, forged intent, changed logical input, or persistence failure.
    pub async fn admit_install(
        &self,
        identity: &AuthenticatedIdentity,
        command: InstallDeployment,
    ) -> Result<DeploymentAdmission, DeploymentError> {
        command
            .command
            .validate(identity, DeploymentOperation::Install)?;
        let declaration =
            recipe_domain::parse_recipe(command.intent.declaration_toml().as_bytes())?;
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(repository_error)?;
        // Current authorization and catalog validation precede command/deployment lookup.
        self.install_authority(&mut tx, identity, &declaration, &command.intent)
            .await?;
        let organization = persistence::lock_project(&mut tx, command.intent.project_id()).await?;
        let evidence = catalog::load(&mut tx, &declaration, &command.intent).await?;
        let intent = catalog::reconstruct(&command.intent, &declaration, &evidence)?;
        if let Some(row) = receipts::find(&mut tx, command.command).await? {
            if row.deployment_id != intent.id().as_uuid()
                || row.input_hash.as_slice() != intent.input_hash().as_bytes()
            {
                return Err(DeploymentError::InputConflict);
            }
            let snapshot = hydration::load(&mut tx, intent.id()).await?;
            intent.validate_replay(&snapshot.intent)?;
            let receipt = receipts::restore(&row, command.command, &snapshot, intent.input_hash())?;
            let disposition = AdmissionDisposition::Resume;
            receipts::attempt(
                &mut tx,
                identity,
                &receipt,
                intent.project_id(),
                disposition,
            )
            .await?;
            tx.commit().await.map_err(repository_error)?;
            return Ok(DeploymentAdmission {
                disposition,
                snapshot,
                receipt,
            });
        }
        // Another actor/command must not adopt an existing deployment or its tombstone.
        if persistence::existing_deployment(&mut tx, &intent)
            .await?
            .is_some()
        {
            return Err(DeploymentError::InputConflict);
        }
        persistence::insert(&mut tx, identity, &intent, &evidence).await?;
        let snapshot = hydration::load(&mut tx, intent.id()).await?;
        let receipt = receipts::append(
            &mut tx,
            identity,
            command.command,
            &snapshot,
            intent.input_hash(),
            organization,
        )
        .await?;
        let disposition = AdmissionDisposition::Created;
        receipts::attempt(
            &mut tx,
            identity,
            &receipt,
            intent.project_id(),
            disposition,
        )
        .await?;
        tx.commit().await.map_err(repository_error)?;
        Ok(DeploymentAdmission {
            disposition,
            snapshot,
            receipt,
        })
    }

    /// Loads safe immutable evidence and current progress under project management.
    ///
    /// # Errors
    /// Rejects unavailable records, denied project access, or malformed stored intent.
    pub async fn inspect(
        &self,
        identity: &AuthenticatedIdentity,
        deployment_id: DeploymentId,
    ) -> Result<DeploymentSnapshot, DeploymentError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(repository_error)?;
        let project = persistence::project(&mut tx, deployment_id).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, project.as_uuid()),
        )
        .await?;
        let snapshot = hydration::load(&mut tx, deployment_id).await?;
        tx.commit().await.map_err(repository_error)?;
        Ok(snapshot)
    }

    /// Admits policy-bounded cleanup without requiring continued source access.
    ///
    /// # Errors
    /// Rejects denied cleanup, changed command input, stale versions, or invalid state.
    pub async fn admit_remove(
        &self,
        identity: &AuthenticatedIdentity,
        command: RemoveDeployment,
    ) -> Result<DeploymentAdmission, DeploymentError> {
        command
            .command
            .validate(identity, DeploymentOperation::Remove)?;
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(repository_error)?;
        let project = persistence::project(&mut tx, command.deployment_id).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, project.as_uuid()),
        )
        .await?;
        let organization = persistence::lock_project(&mut tx, project).await?;
        let mut snapshot = hydration::load(&mut tx, command.deployment_id).await?;
        self.cleanup_authority(&mut tx, identity, &snapshot.intent)
            .await?;
        let input_hash = command.input_hash()?;
        if let Some(row) = receipts::find(&mut tx, command.command).await? {
            // Compare original expected-version input; progress may now be newer.
            let receipt = receipts::restore(&row, command.command, &snapshot, input_hash)?;
            let disposition = AdmissionDisposition::Resume;
            receipts::attempt(&mut tx, identity, &receipt, project, disposition).await?;
            tx.commit().await.map_err(repository_error)?;
            return Ok(DeploymentAdmission {
                disposition,
                snapshot,
                receipt,
            });
        }
        if snapshot.version != command.expected_version {
            return Err(DeploymentError::StaleClaim);
        }
        if matches!(
            snapshot.lifecycle,
            DeploymentLifecycle::Removing | DeploymentLifecycle::Removed
        ) {
            return Err(DeploymentError::InputConflict);
        }
        let affected = sqlx::query(
            "UPDATE recipe_deployments SET lifecycle = 'removing', version = version + 1
             WHERE id = $1 AND version = $2",
        )
        .bind(command.deployment_id.as_uuid())
        .bind(i64::try_from(command.expected_version).map_err(|_| DeploymentError::StaleClaim)?)
        .execute(&mut *tx)
        .await
        .map_err(repository_error)?
        .rows_affected();
        if affected != 1 {
            return Err(DeploymentError::StaleClaim);
        }
        snapshot = hydration::load(&mut tx, command.deployment_id).await?;
        let receipt = receipts::append(
            &mut tx,
            identity,
            command.command,
            &snapshot,
            input_hash,
            organization,
        )
        .await?;
        let disposition = AdmissionDisposition::Created;
        receipts::attempt(&mut tx, identity, &receipt, project, disposition).await?;
        tx.commit().await.map_err(repository_error)?;
        Ok(DeploymentAdmission {
            disposition,
            snapshot,
            receipt,
        })
    }
}
