//! Exact active-claim data; no provider calls or reusable authorization stamps.

mod authority;
pub mod hydration;
mod retained_external;
mod rows;

use async_trait::async_trait;
use authz_postgres::begin_actor_transaction;
use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    ActiveDeploymentEffectRepository, CommandIdentity, DeploymentError, DeploymentId, EffectClaim,
};

use super::{PostgresExecutionPreparationRepository, authority as preparation_authority};
use crate::{effect_context, persistence, repository_error};

#[async_trait]
impl ActiveDeploymentEffectRepository for PostgresExecutionPreparationRepository {
    async fn load_active_effect(
        &self,
        identity: &AuthenticatedIdentity,
        current_command: CommandIdentity,
        deployment: DeploymentId,
        resource: &CapabilitySlotKey,
    ) -> Result<Option<EffectClaim>, DeploymentError> {
        current_command.validate(identity, current_command.operation())?;
        let mut tx = begin_actor_transaction(&self.repository.pool, identity)
            .await
            .map_err(repository_error)?;
        if self.repository.remove_resume.is_some()
            && current_command.operation() == recipe_application::DeploymentOperation::Remove
        {
            crate::remove_resume::context::reserve(&mut tx, deployment).await?;
        }
        preparation_authority::active(&mut tx, identity).await?;
        let project =
            preparation_authority::project(&self.repository, &mut tx, identity, deployment).await?;
        persistence::lock_project(&mut tx, project).await?;
        preparation_authority::lock(&mut tx, deployment).await?;
        preparation_authority::active(&mut tx, identity).await?;
        let snapshot = crate::hydration::load(&mut tx, deployment).await?;
        if let Some(config) = self.repository.remove_resume.as_ref().filter(|_| {
            current_command.operation() == recipe_application::DeploymentOperation::Remove
        }) {
            crate::remove_resume::context::configured(&mut tx, &snapshot.intent, config).await?;
            crate::remove_resume::context::command_configuration(&mut tx, current_command, config)
                .await?;
        }
        authority::current(self, &mut tx, identity, current_command, &snapshot).await?;
        let plan = snapshot
            .intent
            .resources()
            .get(resource)
            .ok_or(DeploymentError::Unavailable)?;
        let state = effect_context::state(&mut tx, deployment, resource).await?;
        let claim = hydration::load(
            &mut tx,
            &snapshot,
            plan,
            resource,
            &state,
            current_command,
            self.repository.remove_resume.as_ref(),
        )
        .await?;
        tx.commit().await.map_err(repository_error)?;
        Ok(claim)
    }
}

impl PostgresExecutionPreparationRepository {
    pub(crate) async fn assert_current_reopening(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        snapshot: &recipe_application::DeploymentSnapshot,
    ) -> Result<(), DeploymentError> {
        preparation_authority::active(tx, identity).await?;
        authority::current(self, tx, identity, command, snapshot).await
    }
    // Removing the old installation checks current backend configuration only;
    // it never borrows old Install or Source permission.
    pub(crate) async fn assert_ready_retention_configuration(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        intent: &recipe_application::DeploymentIntent,
    ) -> Result<(), DeploymentError> {
        let stored = super::hydration::load(tx, intent)
            .await?
            .ok_or(DeploymentError::ReconciliationRequired)?;
        stored
            .preparation
            .validate_configuration(self.profile, &self.platform)
    }
}
