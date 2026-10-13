//! Durable owning-engine mapping, separate from generic volume registration.

pub mod mapping;
mod remove_original;
pub mod storage;

use crate::{PostgresDeploymentRepository, effect_context, effect_outcome, repository_error};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{DeploymentError, EffectClaim, PreparedOwnedVolumeCreation};

impl PostgresDeploymentRepository {
    /// Loads a completed original owned Volume birth under current Remove authority.
    ///
    /// Requires this repository's retained removal configuration and one actor
    /// read-only snapshot. Returned originals are comparison data, not provider
    /// custody, retention permission or physical evidence. Worker discovery and
    /// owning retention must independently requalify them before any effect.
    ///
    /// Supports the actual Ready Create and distinct `creation_retained_ready`
    /// disposition. Other removal chains require their own qualified reader.
    ///
    /// # Errors
    /// Denies missing configuration, authority loss, changed intent, partial or
    /// foreign births, contradictory history and failed transaction rollback.
    pub async fn load_owned_volume_remove_original(
        &self,
        identity: &AuthenticatedIdentity,
        current: recipe_application::CommandIdentity,
        intent: &recipe_application::DeploymentIntent,
        resource: &capability_domain::CapabilitySlotKey,
    ) -> Result<(EffectClaim, PreparedOwnedVolumeCreation), DeploymentError> {
        remove_original::load(self, identity, current, intent, resource).await
    }

    /// Commits original owned-volume provider preparation before registration IO.
    ///
    /// # Errors
    /// Rejects denied current Install authority, stale claims and changed preparation.
    pub async fn prepare_owned_volume_creation(
        &self,
        identity: &AuthenticatedIdentity,
        claim: &EffectClaim,
    ) -> Result<PreparedOwnedVolumeCreation, DeploymentError> {
        let mut context = self
            .execution_context(identity, claim.command, claim.deployment_id, false)
            .await?;
        effect_outcome::exact(&mut context.tx, claim).await?;
        if context.cleanup {
            return Err(DeploymentError::InvalidAction);
        }
        let desired = mapping::derive(&context.snapshot.intent, claim)?;
        if let Some(existing) = storage::load(&mut context.tx, claim).await? {
            if existing != desired {
                return Err(DeploymentError::InputConflict);
            }
            context.tx.commit().await.map_err(repository_error)?;
            return Ok(existing);
        }
        let current =
            effect_context::state(&mut context.tx, claim.deployment_id, &claim.resource).await?;
        if context.cleanup
            || current.active_attempt != Some(claim.provenance.attempt_id.as_uuid())
            || crate::hydration::unsigned(current.version)? != claim.resource_version
            || crate::hydration::unsigned(current.generation)? != claim.generation
        {
            return Err(DeploymentError::StaleClaim);
        }
        storage::append(&mut context, identity, claim, &desired).await?;
        context.tx.commit().await.map_err(repository_error)?;
        Ok(desired)
    }
}
