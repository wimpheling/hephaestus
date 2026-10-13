//! The sole injected failure is a final Ready worker record, all else is actual SQL.

use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::{
    BeginOwnedProvisioning, OwnedBackingObservation, OwnedBackingPhase, OwnedProvisioningClaim,
    OwnedProvisioningContext, VolumeError, VolumeOwnedProvisioningRepository,
};

pub struct Worker {
    pub actual: Arc<PostgresVolumeMetadataRepository>,
    fail_ready: AtomicBool,
}
impl Worker {
    pub const fn new(actual: Arc<PostgresVolumeMetadataRepository>, fail_ready: bool) -> Self {
        Self {
            actual,
            fail_ready: AtomicBool::new(fail_ready),
        }
    }
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
                "fixture failed final Ready SQL record before delegation",
            ));
        }
        self.actual
            .record_owned_observation(claim, observation)
            .await
    }
}
