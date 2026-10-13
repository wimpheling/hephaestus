use async_trait::async_trait;
use runtime_types::{AgentInstanceId, RunId, VolumeId};
use time::OffsetDateTime;
use uuid::Uuid;
use volume_trait::{
    ProvisioningClaim, Volume, VolumeError, VolumeLease, VolumeMetadataRepository,
    VolumeProvisioningState,
};

// Every generic call is a hard failure: owned provisioning must use its explicit ports.
pub struct Legacy;
fn forbidden<T>() -> Result<T, VolumeError> {
    Err(VolumeError::InvalidState(
        "owned test forbids generic metadata",
    ))
}
#[async_trait]
impl VolumeMetadataRepository for Legacy {
    async fn resolve_instance_state(
        &self,
        _: AgentInstanceId,
        _: u64,
        _: &str,
        _: &std::path::Path,
        _: Uuid,
    ) -> Result<Volume, VolumeError> {
        forbidden()
    }
    async fn reserve_provider(
        &self,
        _: VolumeId,
        _: &str,
        _: &std::path::Path,
    ) -> Result<Volume, VolumeError> {
        forbidden()
    }
    async fn volume(&self, _: VolumeId) -> Result<Volume, VolumeError> {
        forbidden()
    }
    async fn claim_provisioning(&self, _: VolumeId) -> Result<ProvisioningClaim, VolumeError> {
        forbidden()
    }
    async fn provisioning_progress(
        &self,
        _: &ProvisioningClaim,
        _: VolumeProvisioningState,
    ) -> Result<(), VolumeError> {
        forbidden()
    }
    async fn mark_ready(&self, _: VolumeId) -> Result<(), VolumeError> {
        forbidden()
    }
    async fn acquire(
        &self,
        _: VolumeId,
        _: RunId,
        _: &str,
        _: OffsetDateTime,
        _: OffsetDateTime,
    ) -> Result<VolumeLease, VolumeError> {
        forbidden()
    }
    async fn mark_attached(
        &self,
        _: &VolumeLease,
        _: OffsetDateTime,
        _: OffsetDateTime,
    ) -> Result<VolumeLease, VolumeError> {
        forbidden()
    }
    async fn heartbeat(
        &self,
        _: &VolumeLease,
        _: OffsetDateTime,
        _: OffsetDateTime,
    ) -> Result<VolumeLease, VolumeError> {
        forbidden()
    }
    async fn active_lease_for_run(
        &self,
        _: RunId,
        _: &str,
    ) -> Result<Option<VolumeLease>, VolumeError> {
        forbidden()
    }
    async fn release_after_detach(&self, _: &VolumeLease, _: bool) -> Result<(), VolumeError> {
        forbidden()
    }
    async fn stale_leases(&self, _: OffsetDateTime) -> Result<Vec<VolumeLease>, VolumeError> {
        forbidden()
    }
    async fn begin_recovery(&self, _: &VolumeLease) -> Result<(), VolumeError> {
        forbidden()
    }
}
