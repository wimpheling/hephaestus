//! Durable provider reservation and generation-fenced provisioning progress.

use std::path::Path;

use runtime_types::{AgentInstanceId, VolumeId};
use uuid::Uuid;
use volume_trait::{ProvisioningClaim, Volume, VolumeError, VolumeProvisioningState};

use crate::{
    PostgresVolumeMetadataRepository,
    errors::{assert_host, invalid, metadata},
    models::VolumeRow,
};

impl PostgresVolumeMetadataRepository {
    pub(super) async fn reserve_legacy(
        &self,
        instance_id: AgentInstanceId,
        capacity_bytes: u64,
        host_id: &str,
        root: &Path,
        filesystem_uuid: Uuid,
    ) -> Result<Volume, VolumeError> {
        let mut tx = self.pool.begin().await.map_err(metadata)?;
        let row = sqlx::query_as::<_, VolumeRow>(
            "SELECT * FROM agent_instance_state_volumes WHERE instance_id=$1 FOR UPDATE",
        )
        .bind(instance_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(metadata)?
        .ok_or(VolumeError::NotFound(VolumeId::from_uuid(Uuid::nil())))?;
        let row = if row.host_id.is_none() {
            let path = expected_path(root, row.id)?;
            sqlx::query_as::<_, VolumeRow>("UPDATE agent_instance_state_volumes SET host_id=$2,host_path=$3,capacity_bytes=$4,filesystem_uuid=COALESCE(filesystem_uuid,$5),updated_at=now() WHERE id=$1 AND host_id IS NULL RETURNING *")
                .bind(row.id).bind(host_id).bind(path)
                .bind(i64::try_from(capacity_bytes).map_err(metadata)?).bind(filesystem_uuid)
                .fetch_one(&mut *tx).await.map_err(metadata)?
        } else {
            validate_reservation(&row, host_id, root)?;
            row
        };
        tx.commit().await.map_err(metadata)?;
        row.try_into()
    }

    pub(super) async fn reserve_standalone(
        &self,
        volume_id: VolumeId,
        host_id: &str,
        root: &Path,
    ) -> Result<Volume, VolumeError> {
        let mut tx = self.pool.begin().await.map_err(metadata)?;
        let row = sqlx::query_as::<_, VolumeRow>(
            "SELECT * FROM agent_instance_state_volumes WHERE id=$1 FOR UPDATE",
        )
        .bind(volume_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(metadata)?
        .ok_or(VolumeError::NotFound(volume_id))?;
        let row = if row.host_id.is_none() {
            if row.filesystem_uuid.is_none() {
                return Err(VolumeError::InvalidState(
                    "volume has no reserved filesystem identity",
                ));
            }
            let path = expected_path(root, row.id)?;
            sqlx::query_as::<_, VolumeRow>("UPDATE agent_instance_state_volumes SET host_id=$2,host_path=$3,updated_at=now() WHERE id=$1 AND host_id IS NULL RETURNING *")
                .bind(row.id).bind(host_id).bind(path).fetch_one(&mut *tx).await.map_err(metadata)?
        } else {
            validate_reservation(&row, host_id, root)?;
            row
        };
        tx.commit().await.map_err(metadata)?;
        row.try_into()
    }

    pub(super) async fn claim(
        &self,
        volume_id: VolumeId,
    ) -> Result<ProvisioningClaim, VolumeError> {
        let row = sqlx::query_as::<_, VolumeRow>("UPDATE agent_instance_state_volumes SET provisioning_generation=provisioning_generation+1,updated_at=now() WHERE id=$1 AND state='uninitialized' AND provisioning_state<>'ready' AND host_id IS NOT NULL AND provisioning_generation<9223372036854775807 RETURNING *")
            .bind(volume_id.as_uuid()).fetch_optional(&self.pool).await.map_err(metadata)?
            .ok_or(VolumeError::InvalidState("volume is not eligible for provisioning"))?;
        Ok(ProvisioningClaim {
            volume: row.try_into()?,
        })
    }

    pub(super) async fn progress(
        &self,
        claim: &ProvisioningClaim,
        state: VolumeProvisioningState,
    ) -> Result<(), VolumeError> {
        let result = sqlx::query("UPDATE agent_instance_state_volumes SET provisioning_state=$3,state=CASE WHEN $3='ready' THEN 'ready' ELSE state END,updated_at=now() WHERE id=$1 AND provisioning_generation=$2 AND state='uninitialized' AND host_id=$4 AND host_path=$5 AND filesystem_uuid=$6 AND capacity_bytes=$7")
            .bind(claim.volume.id.as_uuid()).bind(claim.volume.provisioning_generation)
            .bind(state.as_str()).bind(&claim.volume.host_id)
            .bind(claim.volume.host_path.to_str().ok_or_else(|| invalid("volume path is not UTF-8"))?)
            .bind(claim.volume.filesystem_uuid).bind(i64::try_from(claim.volume.capacity_bytes).map_err(metadata)?)
            .execute(&self.pool).await.map_err(metadata)?;
        if result.rows_affected() != 1 {
            return Err(VolumeError::StaleLease);
        }
        Ok(())
    }
}

fn expected_path(root: &Path, id: Uuid) -> Result<String, VolumeError> {
    if !root.is_absolute()
        || root.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(invalid("provider root must be absolute and canonical"));
    }
    root.join(format!("{id}.raw"))
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("volume path is not UTF-8"))
}

fn validate_reservation(row: &VolumeRow, host_id: &str, root: &Path) -> Result<(), VolumeError> {
    assert_host(
        row.host_id.as_deref().ok_or(VolumeError::IntentConflict)?,
        host_id,
    )?;
    if row.host_path.as_deref() != Some(expected_path(root, row.id)?.as_str()) {
        return Err(VolumeError::IntentConflict);
    }
    Ok(())
}
