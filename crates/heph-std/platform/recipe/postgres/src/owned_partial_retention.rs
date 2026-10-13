//! Atomic actor fence/progress boundary. Actual Local custody belongs to composition.

mod context;
mod transition;

use crate::{
    PostgresDeploymentRepository, execution_transitions, owned_creation_preparation,
    repository_error,
};
use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, EffectClaim, OwnedPartialRetentionRepository, PartialCreationRetentionPlan,
    PreparedOwnedVolumeCreation, ResourceProgress, RetainPartialCreation,
};
use uuid::Uuid;

impl PostgresDeploymentRepository {
    /// Discovers exact owned preparation or a committed result after live cleanup checks.
    ///
    /// # Errors
    /// Rejects denied management, borrowed commands, changed input, stale CAS or lineage.
    pub async fn partial_creation_retention_plan(
        &self,
        identity: &AuthenticatedIdentity,
        request: &RetainPartialCreation,
    ) -> Result<PartialCreationRetentionPlan, DeploymentError> {
        let mut context = context::load(self, identity, request).await?;
        let preparation =
            owned_creation_preparation::storage::load(&mut context.tx, &request.target)
                .await?
                .ok_or(DeploymentError::Unavailable)?;
        if preparation
            != owned_creation_preparation::mapping::derive(
                &context.snapshot.intent,
                &request.target,
            )?
        {
            return Err(DeploymentError::InputConflict);
        }
        let completed = execution_transitions::replay(
            &mut context.tx,
            transition::id(request),
            transition::fingerprint(request)?,
        )
        .await?
        .map(|state| state.progress())
        .transpose()?;
        if completed.is_none() {
            context::check_current(&mut context, request).await?;
        }
        context.tx.commit().await.map_err(repository_error)?;
        Ok(PartialCreationRetentionPlan {
            preparation,
            completed,
        })
    }

    /// Commits the exact worker receipt, permanent birth fence and recipe progress.
    ///
    /// Backend composition must keep actual Local custody held through this transaction.
    ///
    /// # Errors
    /// Rejects denied replay, stale observation/CAS, changed correlation, leases or consumers.
    pub async fn retain_partial_creation(
        &self,
        identity: &AuthenticatedIdentity,
        request: &RetainPartialCreation,
        worker_receipt_id: Uuid,
    ) -> Result<ResourceProgress, DeploymentError> {
        let mut context = context::load(self, identity, request).await?;
        let preparation =
            owned_creation_preparation::storage::load(&mut context.tx, &request.target)
                .await?
                .ok_or(DeploymentError::Unavailable)?;
        if preparation
            != owned_creation_preparation::mapping::derive(
                &context.snapshot.intent,
                &request.target,
            )?
        {
            return Err(DeploymentError::InputConflict);
        }
        if let Some(state) = execution_transitions::replay(
            &mut context.tx,
            transition::id(request),
            transition::fingerprint(request)?,
        )
        .await?
        {
            transition::exact_replay_receipt(&mut context.tx, request, worker_receipt_id).await?;
            context.tx.commit().await.map_err(repository_error)?;
            return state.progress();
        }
        let before = context::check_current(&mut context, request).await?;
        let after =
            transition::commit(&mut context, identity, request, &before, worker_receipt_id).await?;
        context.tx.commit().await.map_err(repository_error)?;
        after.progress()
    }
}

#[async_trait]
impl OwnedPartialRetentionRepository for PostgresDeploymentRepository {
    async fn prepare_owned_volume_creation(
        &self,
        identity: &AuthenticatedIdentity,
        claim: &EffectClaim,
    ) -> Result<PreparedOwnedVolumeCreation, DeploymentError> {
        Self::prepare_owned_volume_creation(self, identity, claim).await
    }
    async fn partial_creation_retention_plan(
        &self,
        identity: &AuthenticatedIdentity,
        request: &RetainPartialCreation,
    ) -> Result<PartialCreationRetentionPlan, DeploymentError> {
        Self::partial_creation_retention_plan(self, identity, request).await
    }
    async fn retain_partial_creation(
        &self,
        identity: &AuthenticatedIdentity,
        request: &RetainPartialCreation,
        worker_receipt_id: Uuid,
    ) -> Result<ResourceProgress, DeploymentError> {
        Self::retain_partial_creation(self, identity, request, worker_receipt_id).await
    }
}
