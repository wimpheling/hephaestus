use super::{PostgresVolumeMetadataRepository, errors::metadata, models::VolumeRow};
use async_trait::async_trait;
use runtime_types::{AgentInstanceId, RunId, VolumeId};
use std::path::Path;
use time::OffsetDateTime;
use uuid::Uuid;
use volume_trait::{Volume, VolumeError, VolumeLease, VolumeMetadataRepository};

mod profiled_scalar;
mod public107;
mod scalar_history;

use crate::ScalarMetadataSchema;

#[async_trait]
impl VolumeMetadataRepository for PostgresVolumeMetadataRepository {
    async fn resolve_instance_state(
        &self,
        instance_id: AgentInstanceId,
        capacity_bytes: u64,
        host_id: &str,
        host_path: &Path,
        filesystem_uuid: Uuid,
    ) -> Result<Volume, VolumeError> {
        self.reserve_legacy(
            instance_id,
            capacity_bytes,
            host_id,
            host_path.parent().unwrap_or(host_path),
            filesystem_uuid,
        )
        .await
    }

    async fn reserve_provider(
        &self,
        volume_id: VolumeId,
        host_id: &str,
        root: &Path,
    ) -> Result<Volume, VolumeError> {
        self.reserve_standalone(volume_id, host_id, root).await
    }

    async fn claim_provisioning(
        &self,
        volume_id: VolumeId,
    ) -> Result<volume_trait::ProvisioningClaim, VolumeError> {
        self.claim(volume_id).await
    }

    async fn provisioning_progress(
        &self,
        claim: &volume_trait::ProvisioningClaim,
        state: volume_trait::VolumeProvisioningState,
    ) -> Result<(), VolumeError> {
        self.progress(claim, state).await
    }

    async fn mark_ready(&self, volume_id: VolumeId) -> Result<(), VolumeError> {
        // Compatibility entry point verifies prior fenced provisioning only;
        // attachment and lease recovery must never be reset through this API.
        let ready: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_instance_state_volumes WHERE id=$1 AND state='ready' AND provisioning_state='ready')")
            .bind(volume_id.as_uuid()).fetch_one(&self.pool).await.map_err(metadata)?;
        if !ready {
            return Err(VolumeError::StaleLease);
        }
        Ok(())
    }

    async fn volume(&self, volume_id: VolumeId) -> Result<Volume, VolumeError> {
        sqlx::query_as::<_, VolumeRow>("SELECT * FROM agent_instance_state_volumes WHERE id = $1")
            .bind(volume_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(metadata)?
            .ok_or(VolumeError::NotFound(volume_id))?
            .try_into()
    }

    async fn acquire(
        &self,
        volume_id: VolumeId,
        run_id: RunId,
        host_id: &str,
        now: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<VolumeLease, VolumeError> {
        match self.scalar_schema {
            ScalarMetadataSchema::Public107 => {
                public107::Public107(self)
                    .acquire(volume_id, run_id, host_id, now, expires_at)
                    .await
            }
            ScalarMetadataSchema::Profiled => {
                profiled_scalar::acquire(self, volume_id, run_id, host_id, now, expires_at).await
            }
        }
    }

    async fn mark_attached(
        &self,
        lease: &VolumeLease,
        now: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<VolumeLease, VolumeError> {
        match self.scalar_schema {
            ScalarMetadataSchema::Public107 => {
                public107::Public107(self)
                    .mark_attached(lease, now, expires_at)
                    .await
            }
            ScalarMetadataSchema::Profiled => {
                profiled_scalar::refresh(self, lease, now, expires_at, true).await
            }
        }
    }

    async fn heartbeat(
        &self,
        lease: &VolumeLease,
        now: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<VolumeLease, VolumeError> {
        match self.scalar_schema {
            ScalarMetadataSchema::Public107 => {
                public107::Public107(self)
                    .heartbeat(lease, now, expires_at)
                    .await
            }
            ScalarMetadataSchema::Profiled => {
                profiled_scalar::refresh(self, lease, now, expires_at, false).await
            }
        }
    }

    async fn scalar_lease_history(
        &self,
        run_id: RunId,
    ) -> Result<volume_trait::ScalarLeaseHistory, VolumeError> {
        scalar_history::observe(self, run_id).await
    }
    async fn active_lease_for_run(
        &self,
        run_id: RunId,
        host_id: &str,
    ) -> Result<Option<VolumeLease>, VolumeError> {
        match self.scalar_schema {
            ScalarMetadataSchema::Public107 => {
                public107::Public107(self)
                    .active_lease_for_run(run_id, host_id)
                    .await
            }
            ScalarMetadataSchema::Profiled => profiled_scalar::active(self, run_id, host_id).await,
        }
    }

    async fn release_after_detach(
        &self,
        lease: &VolumeLease,
        recovering: bool,
    ) -> Result<(), VolumeError> {
        match self.scalar_schema {
            ScalarMetadataSchema::Public107 => {
                public107::Public107(self)
                    .release_after_detach(lease, recovering)
                    .await
            }
            ScalarMetadataSchema::Profiled => {
                profiled_scalar::release(self, lease, recovering).await
            }
        }
    }

    async fn stale_leases(&self, now: OffsetDateTime) -> Result<Vec<VolumeLease>, VolumeError> {
        match self.scalar_schema {
            ScalarMetadataSchema::Public107 => public107::Public107(self).stale_leases(now).await,
            ScalarMetadataSchema::Profiled => profiled_scalar::stale(self, now).await,
        }
    }

    async fn begin_recovery(&self, lease: &VolumeLease) -> Result<(), VolumeError> {
        match self.scalar_schema {
            ScalarMetadataSchema::Public107 => {
                public107::Public107(self).begin_recovery(lease).await
            }
            ScalarMetadataSchema::Profiled => profiled_scalar::recover(self, lease).await,
        }
    }
}
