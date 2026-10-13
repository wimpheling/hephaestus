//! Exact-run extension. These queries require the unpublished 0108 schema.
//!
//! Legacy application startup does not compose this extension. Named dispatch
//! remains unsupported until the complete runtime and cleanup path is verified.

mod catalog;
mod invocation_runtime;
mod lease_rows;
pub mod mutations;
mod reservations119;

use async_trait::async_trait;
use runtime_types::RunId;
use time::OffsetDateTime;
use volume_domain::RunVolumeSelections;
use volume_trait::{RunVolumeLease, RunVolumeMetadataRepository, VolumeError};

use super::{PostgresVolumeMetadataRepository, errors::metadata};

#[async_trait]
impl RunVolumeMetadataRepository for PostgresVolumeMetadataRepository {
    async fn load_run_selections(&self, run_id: RunId) -> Result<RunVolumeSelections, VolumeError> {
        if self.invocation_run(run_id).await? {
            return self.invocation_selections(run_id).await;
        }
        let mut tx = self.pool.begin().await.map_err(metadata)?;
        let selections = catalog::load(&mut tx, run_id).await?;
        tx.commit().await.map_err(metadata)?;
        Ok(selections)
    }

    async fn preflight_run(&self, selections: &RunVolumeSelections) -> Result<(), VolumeError> {
        if self.invocation_run(selections.identity().run_id()).await? {
            return self.invocation_check(selections).await;
        }
        let mut tx = self.pool.begin().await.map_err(metadata)?;
        catalog::check(&mut tx, selections).await?;
        tx.commit().await.map_err(metadata)
    }

    async fn acquire_run(
        &self,
        selections: &RunVolumeSelections,
        host_id: &str,
        now: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        if self.invocation_run(selections.identity().run_id()).await? {
            return self
                .invocation_acquire(selections, host_id, now, expires_at)
                .await;
        }
        mutations::acquire(&self.pool, selections, host_id, now, expires_at).await
    }

    async fn mark_run_attached(
        &self,
        selections: &RunVolumeSelections,
        host_id: &str,
        now: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        if self.invocation_run(selections.identity().run_id()).await? {
            return self
                .invocation_refresh(selections, host_id, now, expires_at, true)
                .await;
        }
        mutations::refresh(
            &self.pool,
            selections,
            host_id,
            now,
            expires_at,
            mutations::Refresh::Attached,
        )
        .await
    }

    async fn heartbeat_run(
        &self,
        selections: &RunVolumeSelections,
        host_id: &str,
        now: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        if self.invocation_run(selections.identity().run_id()).await? {
            return self
                .invocation_refresh(selections, host_id, now, expires_at, false)
                .await;
        }
        mutations::refresh(
            &self.pool,
            selections,
            host_id,
            now,
            expires_at,
            mutations::Refresh::Heartbeat,
        )
        .await
    }

    async fn leases_for_run(
        &self,
        run_id: RunId,
        host_id: &str,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        if self.invocation_run(run_id).await? {
            return self.invocation_leases(run_id, host_id).await;
        }
        let mut tx = self.pool.begin().await.map_err(metadata)?;
        lease_rows::lock_consumer_run(&mut tx, run_id).await?;
        let rows = lease_rows::load(&mut tx, run_id, host_id).await?;
        tx.commit().await.map_err(metadata)?;
        Ok(rows)
    }
}
