use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use sqlx::PgPool;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::{
    BeginOwnedProvisioning, OwnedBackingObservation, OwnedBackingPhase,
    OwnedPartialBirthObservation, OwnedPartialRetentionContext, OwnedPartialRetentionReceipt,
    OwnedProvisioningClaim, OwnedProvisioningContext, VolumeError,
    VolumeOwnedProvisioningRepository, VolumePartialRetentionRepository,
};

/// Actual SQL worker with exactly one explicit fault at a real physical phase.
pub struct Worker {
    pub actual: Arc<PostgresVolumeMetadataRepository>,
    pub pool: PgPool,
    pub creator: identity_domain::UserId,
    pub release: uuid::Uuid,
    pub revoke_recorded: AtomicBool,
    pub fail_ready: AtomicBool,
}
#[async_trait]
impl VolumeOwnedProvisioningRepository for Worker {
    async fn begin_owned_provisioning(
        &self,
        identity: &AuthenticatedIdentity,
        request: &BeginOwnedProvisioning,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        self.actual
            .begin_owned_provisioning(identity, request)
            .await
    }
    async fn owned_reconciliation_context(
        &self,
        claim: &OwnedProvisioningClaim,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        self.actual.owned_reconciliation_context(claim).await
    }
    async fn record_owned_observation(
        &self,
        claim: &OwnedProvisioningClaim,
        observation: &OwnedBackingObservation,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        if observation.fields().phase == OwnedBackingPhase::Ready
            && self.fail_ready.swap(false, Ordering::SeqCst)
        {
            return Err(VolumeError::ProvisioningUncertain(
                "fixture loses final physical Ready SQL record",
            ));
        }
        let recorded = self
            .actual
            .record_owned_observation(claim, observation)
            .await?;
        if observation.fields().phase == OwnedBackingPhase::Recorded
            && self.revoke_recorded.swap(false, Ordering::SeqCst)
        {
            sqlx::query("DELETE FROM project_maintainers WHERE user_id=$1")
                .bind(self.creator.as_uuid())
                .execute(&self.pool)
                .await
                .map_err(|error| VolumeError::Metadata(Box::new(error)))?;
            sqlx::query("UPDATE releases SET state='revoked',revoked_at=now() WHERE id=$1")
                .bind(self.release)
                .execute(&self.pool)
                .await
                .map_err(|error| VolumeError::Metadata(Box::new(error)))?;
        }
        Ok(recorded)
    }
}

/// Gates one actual fresh admission result before Local obtains its native flock.
pub struct PausedActor {
    pub actual: Arc<PostgresVolumeMetadataRepository>,
    pub admitted: tokio::sync::Notify,
    pub resume: tokio::sync::Notify,
    pub first: AtomicBool,
}
#[async_trait]
impl VolumeOwnedProvisioningRepository for PausedActor {
    async fn begin_owned_provisioning(
        &self,
        identity: &AuthenticatedIdentity,
        request: &BeginOwnedProvisioning,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        let context = self
            .actual
            .begin_owned_provisioning(identity, request)
            .await?;
        if self.first.swap(false, Ordering::SeqCst) {
            self.admitted.notify_one();
            self.resume.notified().await;
        }
        Ok(context)
    }
    async fn owned_reconciliation_context(
        &self,
        claim: &OwnedProvisioningClaim,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        self.actual.owned_reconciliation_context(claim).await
    }
    async fn record_owned_observation(
        &self,
        claim: &OwnedProvisioningClaim,
        observation: &OwnedBackingObservation,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        self.actual
            .record_owned_observation(claim, observation)
            .await
    }
}

/// Pauses after a real worker receipt, while the coordinator owns native custody.
pub struct PausedPartialWorker {
    pub actual: Arc<PostgresVolumeMetadataRepository>,
    pub recorded: tokio::sync::Notify,
    pub resume: tokio::sync::Notify,
}
#[async_trait]
impl VolumePartialRetentionRepository for PausedPartialWorker {
    async fn partial_retention_context(
        &self,
        claim: &OwnedProvisioningClaim,
    ) -> Result<OwnedPartialRetentionContext, VolumeError> {
        self.actual.partial_retention_context(claim).await
    }

    async fn record_partial_retention_observation(
        &self,
        context: &OwnedPartialRetentionContext,
        observation: &OwnedPartialBirthObservation,
    ) -> Result<OwnedPartialRetentionReceipt, VolumeError> {
        let receipt = self
            .actual
            .record_partial_retention_observation(context, observation)
            .await?;
        self.recorded.notify_one();
        self.resume.notified().await;
        Ok(receipt)
    }
}
