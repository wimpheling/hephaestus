//! Explicit119 worker metadata route. Configuration is data, not physical proof.

use release_service::InstanceExecutionConfiguration;
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use volume_domain::RunVolumeSelections;
use volume_trait::{RunVolumeLease, VolumeError};

use super::{catalog, lease_rows};
use crate::{PostgresVolumeMetadataRepository, errors::metadata};

impl PostgresVolumeMetadataRepository {
    // Immutable-kind routing does not replace ordinary role/error semantics.
    // Missing and non-Invocation rows fall through to the unchanged old path.
    pub(super) async fn invocation_run(&self, run: RunId) -> Result<bool, VolumeError> {
        let kind: Option<String> = sqlx::query_scalar("SELECT run_kind FROM runs WHERE id=$1")
            .bind(run.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(metadata)?;
        if kind.as_deref() != Some("invocation") {
            return Ok(false);
        }
        let mut tx = self.pool.begin().await.map_err(metadata)?;
        require_worker(&mut tx).await?;
        self.invocation_config()?;
        tx.commit().await.map_err(metadata)?;
        Ok(true)
    }

    fn invocation_config(&self) -> Result<&InstanceExecutionConfiguration, VolumeError> {
        self.invocation_configuration
            .as_ref()
            .ok_or(VolumeError::InvalidState("Invocation configuration absent"))
    }

    async fn invocation_transaction(
        &self,
        run: RunId,
    ) -> Result<Transaction<'_, Postgres>, VolumeError> {
        let config =
            serde_json::to_vec(&self.invocation_config()?.canonical_json()).map_err(metadata)?;
        let mut tx = self.pool.begin().await.map_err(metadata)?;
        require_worker(&mut tx).await?;
        super::reservations119::acquire(&mut tx, run).await?;
        sqlx::query("SELECT lock_qualified_invocation119($1,$2)")
            .bind(run.as_uuid())
            .bind(config)
            .execute(&mut *tx)
            .await
            .map_err(metadata)?;
        Ok(tx)
    }

    pub(super) async fn invocation_selections(
        &self,
        run: RunId,
    ) -> Result<RunVolumeSelections, VolumeError> {
        let mut tx = self.invocation_transaction(run).await?;
        let selections = catalog::load_locked(&mut tx, run, true).await?;
        tx.commit().await.map_err(metadata)?;
        Ok(selections)
    }

    pub(super) async fn invocation_check(
        &self,
        expected: &RunVolumeSelections,
    ) -> Result<(), VolumeError> {
        if self
            .invocation_selections(expected.identity().run_id())
            .await?
            != *expected
        {
            return Err(VolumeError::InvalidState("Invocation selections changed"));
        }
        Ok(())
    }

    pub(super) async fn invocation_acquire(
        &self,
        expected: &RunVolumeSelections,
        host: &str,
        now: OffsetDateTime,
        expires: OffsetDateTime,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        let config = self.invocation_interval(host, now, expires)?;
        let run = expected.identity().run_id();
        let mut tx = self.invocation_transaction(run).await?;
        let selections = catalog::load_locked(&mut tx, run, true).await?;
        if selections != *expected {
            return Err(VolumeError::InvalidState(
                "Invocation selection input differs",
            ));
        }
        sqlx::query("SELECT acquire_qualified_invocation119($1,$2,$3,$4,$5,$6)")
            .bind(run.as_uuid())
            .bind(config)
            .bind(serde_json::to_value(expected).map_err(metadata)?)
            .bind(host)
            .bind(now)
            .bind(expires)
            .execute(&mut *tx)
            .await
            .map_err(metadata)?;
        let leases = lease_rows::load_nowait(&mut tx, run, host).await?;
        tx.commit().await.map_err(metadata)?;
        Ok(leases)
    }

    pub(super) async fn invocation_refresh(
        &self,
        expected: &RunVolumeSelections,
        host: &str,
        now: OffsetDateTime,
        expires: OffsetDateTime,
        attached: bool,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        let config = self.invocation_interval(host, now, expires)?;
        let run = expected.identity().run_id();
        let mut tx = self.invocation_transaction(run).await?;
        sqlx::query("SELECT refresh_qualified_invocation119($1,$2,$3,$4,$5,$6,$7)")
            .bind(run.as_uuid())
            .bind(config)
            .bind(serde_json::to_value(expected).map_err(metadata)?)
            .bind(host)
            .bind(now)
            .bind(expires)
            .bind(attached)
            .execute(&mut *tx)
            .await
            .map_err(metadata)?;
        let leases = lease_rows::load_nowait(&mut tx, run, host).await?;
        tx.commit().await.map_err(metadata)?;
        Ok(leases)
    }

    // Immutable held inventory is available after execution authority loss.
    // Reading it conveys no release permission or physical cleanup evidence.
    pub(super) async fn invocation_leases(
        &self,
        run: RunId,
        host: &str,
    ) -> Result<Vec<RunVolumeLease>, VolumeError> {
        if host != self.invocation_config()?.provider_scope().host_id() {
            return Err(VolumeError::InvalidState(
                "Invocation inventory host differs",
            ));
        }
        let mut tx = self.invocation_transaction(run).await?;
        let leases = lease_rows::load_nowait(&mut tx, run, host).await?;
        tx.commit().await.map_err(metadata)?;
        Ok(leases)
    }

    fn invocation_interval(
        &self,
        host: &str,
        now: OffsetDateTime,
        expires: OffsetDateTime,
    ) -> Result<Vec<u8>, VolumeError> {
        let config = self.invocation_config()?;
        if host != config.provider_scope().host_id() || expires <= now {
            return Err(VolumeError::InvalidState(
                "Invocation host or interval differs",
            ));
        }
        serde_json::to_vec(&config.canonical_json()).map_err(metadata)
    }
}

async fn require_worker(tx: &mut Transaction<'_, Postgres>) -> Result<(), VolumeError> {
    let worker: bool = sqlx::query_scalar("SELECT current_user='hephaestus_worker'")
        .fetch_one(&mut **tx)
        .await
        .map_err(metadata)?;
    if !worker {
        return Err(VolumeError::InvalidState(
            "Invocation metadata requires worker role",
        ));
    }
    Ok(())
}
