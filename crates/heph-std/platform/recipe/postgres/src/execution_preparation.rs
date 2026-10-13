//! Whole-graph input preparation without registration, import or provider IO.

pub mod active_effect;
mod authority;
pub mod hydration;
mod reservation;
mod rows;
mod storage;

use async_trait::async_trait;
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentExecutionPreparation,
    DeploymentExecutionPreparationRepository, DeploymentExecutionProfile, DeploymentId,
    DeploymentOperation, ExecutionPreparationAdmission, PlatformPolicyObservation,
};
use sqlx::PgPool;

use crate::{PostgresDeploymentRepository, persistence, repository_error};

/// Authoritative preparation with policy/profile selected by server composition.
///
/// This adapter commits metadata only. Every later effect independently checks
/// fresh authority and the exact prepared input before touching a provider.
#[derive(Clone)]
pub struct PostgresExecutionPreparationRepository {
    repository: PostgresDeploymentRepository,
    platform: PlatformPolicyObservation,
    profile: DeploymentExecutionProfile,
}

impl PostgresExecutionPreparationRepository {
    /// Enables exact old removal-claim hydration under fresh qualified admission.
    ///
    /// # Errors
    /// Rejects a configuration different from this adapter's prepared semantics.
    pub fn with_remove_resume(
        mut self,
        config: recipe_application::RemoveResumeConfiguration,
    ) -> Result<Self, DeploymentError> {
        if config.profile() != self.profile || config.platform() != &self.platform {
            return Err(DeploymentError::IntentMismatch);
        }
        self.repository = self.repository.with_remove_resume(config);
        Ok(self)
    }

    /// Creates a backend adapter with trusted configured policy and semantics.
    #[must_use]
    pub const fn new(
        pool: PgPool,
        platform: PlatformPolicyObservation,
        profile: DeploymentExecutionProfile,
    ) -> Self {
        Self {
            repository: PostgresDeploymentRepository::new(pool),
            platform,
            profile,
        }
    }
}

#[async_trait]
impl DeploymentExecutionPreparationRepository for PostgresExecutionPreparationRepository {
    async fn prepare_execution(
        &self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        deployment: DeploymentId,
    ) -> Result<ExecutionPreparationAdmission, DeploymentError> {
        command.validate(identity, DeploymentOperation::Install)?;
        let mut tx = begin_actor_transaction(&self.repository.pool, identity)
            .await
            .map_err(repository_error)?;
        authority::active(&mut tx, identity).await?;
        let project = authority::project(&self.repository, &mut tx, identity, deployment).await?;
        let prospective =
            reservation::prospective(self, &mut tx, identity, command, deployment).await?;
        persistence::lock_project(&mut tx, project).await?;
        authority::lock(&mut tx, deployment).await?;
        authority::active(&mut tx, identity).await?;
        authority::install_open(&mut tx, deployment).await?;
        let snapshot = crate::hydration::load(&mut tx, deployment).await?;
        let original = authority::original(&mut tx, &snapshot.intent).await?;
        if command != original {
            return Err(DeploymentError::InputConflict);
        }
        let declaration =
            recipe_domain::parse_recipe(snapshot.intent.declaration_toml().as_bytes())?;
        self.repository
            .install_authority(&mut tx, identity, &declaration, &snapshot.intent)
            .await?;
        let existing = hydration::load(&mut tx, &snapshot.intent).await?;
        if let Some(stored) = &existing {
            stored
                .preparation
                .validate_configuration(self.profile, &self.platform)?;
        } else {
            authority::before_effects(&mut tx, &snapshot).await?;
        }
        let sources =
            authority::sources(&mut tx, &declaration, &snapshot.intent, &self.platform).await?;
        let desired = DeploymentExecutionPreparation::new(
            &snapshot.intent,
            original,
            self.profile,
            self.platform.clone(),
            &sources,
        )?;
        if desired != prospective {
            return Err(DeploymentError::InputConflict);
        }
        if existing
            .as_ref()
            .is_some_and(|stored| stored.preparation != desired)
        {
            return Err(DeploymentError::InputConflict);
        }
        let written = storage::write(&mut tx, &desired).await?;
        let mut result = hydration::load(&mut tx, &snapshot.intent)
            .await?
            .ok_or(DeploymentError::IntentMismatch)?;
        if result.preparation != desired || result.receipt.event != written.event {
            return Err(DeploymentError::IntentMismatch);
        }
        result.replayed = written.replayed;
        tx.commit().await.map_err(repository_error)?;
        Ok(result)
    }

    async fn inspect_execution_preparation(
        &self,
        identity: &AuthenticatedIdentity,
        deployment: DeploymentId,
    ) -> Result<Option<DeploymentExecutionPreparation>, DeploymentError> {
        let mut tx = begin_actor_transaction(&self.repository.pool, identity)
            .await
            .map_err(repository_error)?;
        authority::project(&self.repository, &mut tx, identity, deployment).await?;
        let snapshot = crate::hydration::load(&mut tx, deployment).await?;
        let preparation = hydration::load(&mut tx, &snapshot.intent)
            .await?
            .map(|stored| stored.preparation);
        tx.commit().await.map_err(repository_error)?;
        Ok(preparation)
    }
}
