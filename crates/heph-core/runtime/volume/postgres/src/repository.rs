use super::{
    PostgresVolumeMetadataRepository,
    errors::{assert_host, invalid, metadata},
    models::{LeaseRow, VolumeRow},
};
use async_trait::async_trait;
use runtime_types::{AgentInstanceId, LeaseId, RunId, VolumeId};
use sqlx::{Postgres, Transaction};
use std::path::Path;
use time::OffsetDateTime;
use uuid::Uuid;
use volume_trait::{Volume, VolumeError, VolumeLease, VolumeMetadataRepository};

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
        let mut tx = self.pool.begin().await.map_err(metadata)?;
        let existing = sqlx::query_as::<_, VolumeRow>(
            "SELECT * FROM agent_instance_state_volumes WHERE instance_id = $1 FOR UPDATE",
        )
        .bind(instance_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(metadata)?;
        let Some(row) = existing else {
            return Err(VolumeError::NotFound(VolumeId::from_uuid(Uuid::nil())));
        };
        let row = if row.state == "uninitialized" {
            let capacity = i64::try_from(capacity_bytes).map_err(metadata)?;
            let root = host_path.parent().unwrap_or(host_path);
            let path = root.join(format!("{}.raw", row.id));
            let path = path
                .to_str()
                .ok_or_else(|| invalid("volume path is not UTF-8"))?;
            sqlx::query_as::<_, VolumeRow>(
                "UPDATE agent_instance_state_volumes SET host_id = $2, host_path = $3,
                 capacity_bytes = $4, filesystem_uuid = $5, updated_at = now()
                 WHERE id = $1 AND state = 'uninitialized' RETURNING *",
            )
            .bind(row.id)
            .bind(host_id)
            .bind(path)
            .bind(capacity)
            .bind(filesystem_uuid)
            .fetch_one(&mut *tx)
            .await
            .map_err(metadata)?
        } else {
            row
        };
        tx.commit().await.map_err(metadata)?;
        row.try_into()
    }

    async fn mark_ready(&self, volume_id: VolumeId) -> Result<(), VolumeError> {
        let result = sqlx::query("UPDATE agent_instance_state_volumes SET state = 'ready', updated_at = now() WHERE id = $1 AND state = 'uninitialized'")
            .bind(volume_id.as_uuid()).execute(&self.pool).await.map_err(metadata)?;
        if result.rows_affected() == 0 {
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
        let mut tx = self.pool.begin().await.map_err(metadata)?;
        let volume = sqlx::query_as::<_, VolumeRow>(
            "SELECT * FROM agent_instance_state_volumes WHERE id = $1 FOR UPDATE",
        )
        .bind(volume_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(metadata)?
        .ok_or(VolumeError::NotFound(volume_id))?;
        assert_host(
            volume
                .host_id
                .as_deref()
                .ok_or(VolumeError::InvalidState("volume has no host owner"))?,
            host_id,
        )?;
        if let Some(existing) = active_lease(&mut tx, volume_id).await? {
            if existing.run_id == run_id.as_uuid() {
                tx.commit().await.map_err(metadata)?;
                return existing.try_into();
            }
            return Err(VolumeError::LeaseConflict {
                volume_id,
                holder_run_id: RunId::from_uuid(existing.run_id),
            });
        }
        if volume.state != "ready" {
            return Err(VolumeError::InvalidState(
                "only a ready volume can be leased",
            ));
        }
        let generation = volume
            .lease_generation
            .checked_add(1)
            .ok_or(VolumeError::InvalidState("lease generation overflow"))?;
        let lease_id = LeaseId::new();
        sqlx::query("INSERT INTO agent_instance_volume_leases (id, volume_id, instance_id, run_id, host_id, fencing_token, state, acquired_at, heartbeat_at, expires_at) VALUES ($1,$2,$3,$4,$5,$6,'active',$7,$7,$8)")
            .bind(lease_id.as_uuid()).bind(volume_id.as_uuid()).bind(volume.instance_id).bind(run_id.as_uuid()).bind(host_id).bind(generation).bind(now).bind(expires_at)
            .execute(&mut *tx).await.map_err(metadata)?;
        sqlx::query("UPDATE agent_instance_state_volumes SET lease_generation = $2, updated_at = now() WHERE id = $1")
            .bind(volume_id.as_uuid()).bind(generation).execute(&mut *tx).await.map_err(metadata)?;
        tx.commit().await.map_err(metadata)?;
        self.lease(lease_id).await
    }

    async fn mark_attached(
        &self,
        lease: &VolumeLease,
        now: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<VolumeLease, VolumeError> {
        let result = sqlx::query("WITH current_lease AS (UPDATE agent_instance_volume_leases SET attached_at = COALESCE(attached_at,$4), heartbeat_at=$4, expires_at=$5 WHERE id=$1 AND volume_id=$2 AND fencing_token=$3 AND released_at IS NULL RETURNING volume_id) UPDATE agent_instance_state_volumes SET state='attached', updated_at=$4 WHERE id IN (SELECT volume_id FROM current_lease) AND state IN ('ready','attached')")
            .bind(lease.id.as_uuid()).bind(lease.volume_id.as_uuid()).bind(lease.fencing_token).bind(now).bind(expires_at).execute(&self.pool).await.map_err(metadata)?;
        if result.rows_affected() == 0 {
            return Err(VolumeError::StaleLease);
        }
        self.lease(lease.id).await
    }

    async fn heartbeat(
        &self,
        lease: &VolumeLease,
        now: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<VolumeLease, VolumeError> {
        let result = sqlx::query("UPDATE agent_instance_volume_leases SET heartbeat_at=$4, expires_at=$5 WHERE id=$1 AND volume_id=$2 AND fencing_token=$3 AND released_at IS NULL AND recovering_at IS NULL")
            .bind(lease.id.as_uuid()).bind(lease.volume_id.as_uuid()).bind(lease.fencing_token).bind(now).bind(expires_at).execute(&self.pool).await.map_err(metadata)?;
        if result.rows_affected() == 0 {
            return Err(VolumeError::StaleLease);
        }
        self.lease(lease.id).await
    }

    async fn active_lease_for_run(
        &self,
        run_id: RunId,
        host_id: &str,
    ) -> Result<Option<VolumeLease>, VolumeError> {
        let row = sqlx::query_as::<_, LeaseRow>(
            "SELECT * FROM agent_instance_volume_leases WHERE run_id=$1 AND released_at IS NULL",
        )
        .bind(run_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(metadata)?;
        row.map(|row| {
            assert_host(&row.host_id, host_id)?;
            row.try_into()
        })
        .transpose()
    }

    async fn release_after_detach(
        &self,
        lease: &VolumeLease,
        recovering: bool,
    ) -> Result<(), VolumeError> {
        let mut tx = self.pool.begin().await.map_err(metadata)?;
        let result = if recovering {
            sqlx::query("UPDATE agent_instance_volume_leases SET released_at=now(), state='released' WHERE id=$1 AND volume_id=$2 AND fencing_token=$3 AND released_at IS NULL AND recovering_at IS NOT NULL")
                .bind(lease.id.as_uuid())
                .bind(lease.volume_id.as_uuid())
                .bind(lease.fencing_token)
                .execute(&mut *tx)
                .await
        } else {
            sqlx::query("UPDATE agent_instance_volume_leases SET released_at=now(), state='released' WHERE id=$1 AND volume_id=$2 AND fencing_token=$3 AND released_at IS NULL AND recovering_at IS NULL")
                .bind(lease.id.as_uuid())
                .bind(lease.volume_id.as_uuid())
                .bind(lease.fencing_token)
                .execute(&mut *tx)
                .await
        }
        .map_err(metadata)?;
        if result.rows_affected() == 0 {
            return Err(VolumeError::StaleLease);
        }
        sqlx::query("UPDATE agent_instance_state_volumes SET state='ready', updated_at=now() WHERE id=$1 AND lease_generation=$2").bind(lease.volume_id.as_uuid()).bind(lease.fencing_token).execute(&mut *tx).await.map_err(metadata)?;
        tx.commit().await.map_err(metadata)
    }

    async fn stale_leases(&self, now: OffsetDateTime) -> Result<Vec<VolumeLease>, VolumeError> {
        sqlx::query_as::<_, LeaseRow>("SELECT * FROM agent_instance_volume_leases WHERE released_at IS NULL AND expires_at <= $1 ORDER BY expires_at,id").bind(now).fetch_all(&self.pool).await.map_err(metadata)?.into_iter().map(TryInto::try_into).collect()
    }

    async fn begin_recovery(&self, lease: &VolumeLease) -> Result<(), VolumeError> {
        let result = sqlx::query("WITH current_lease AS (UPDATE agent_instance_volume_leases SET recovering_at=COALESCE(recovering_at,now()) WHERE id=$1 AND volume_id=$2 AND fencing_token=$3 AND released_at IS NULL AND expires_at <= now() RETURNING volume_id) UPDATE agent_instance_state_volumes SET state='recovering', updated_at=now() WHERE id IN (SELECT volume_id FROM current_lease)")
            .bind(lease.id.as_uuid()).bind(lease.volume_id.as_uuid()).bind(lease.fencing_token).execute(&self.pool).await.map_err(metadata)?;
        if result.rows_affected() == 0 {
            return Err(VolumeError::StaleLease);
        }
        Ok(())
    }
}

impl PostgresVolumeMetadataRepository {
    async fn lease(&self, id: LeaseId) -> Result<VolumeLease, VolumeError> {
        sqlx::query_as::<_, LeaseRow>("SELECT * FROM agent_instance_volume_leases WHERE id=$1")
            .bind(id.as_uuid())
            .fetch_one(&self.pool)
            .await
            .map_err(metadata)?
            .try_into()
    }
}

async fn active_lease(
    tx: &mut Transaction<'_, Postgres>,
    volume_id: VolumeId,
) -> Result<Option<LeaseRow>, VolumeError> {
    sqlx::query_as::<_, LeaseRow>("SELECT * FROM agent_instance_volume_leases WHERE volume_id=$1 AND released_at IS NULL FOR UPDATE").bind(volume_id.as_uuid()).fetch_optional(&mut **tx).await.map_err(metadata)
}
