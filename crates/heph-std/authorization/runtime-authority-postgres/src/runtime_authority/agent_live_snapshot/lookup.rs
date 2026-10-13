//! Exact persisted ceilings for trusted owned-profile app checks.

use runtime_authority::{LiveRunSessionCeiling, LiveRunSnapshotPins, RuntimeAuthorityError};

use super::{context, session};
use crate::runtime_authority::{agent_repository::PgRuntimeSessionRepository, common::storage};

impl PgRuntimeSessionRepository {
    /// Loads the stable ceiling of an already recorded owned-run session.
    ///
    /// Returns `None` only when both session and snapshot are absent. This read
    /// grants no source rights, issuance, dispatch, or new runtime effects.
    ///
    /// # Errors
    ///
    /// Rejects disabled configuration, non-worker callers, wrong run pins,
    /// orphaned or inconsistent records, and revoked or elapsed sessions.
    pub async fn recorded_live_session_ceiling(
        &self,
        pins: LiveRunSnapshotPins,
    ) -> Result<Option<LiveRunSessionCeiling>, RuntimeAuthorityError> {
        if !self.canonical_live_snapshots && self.invocation_configuration.is_none() {
            return Err(RuntimeAuthorityError::Unsupported);
        }
        if self.qualified_invocation_run(pins.run()).await? {
            let mut tx = self.pool.begin().await.map_err(storage)?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
            let recorded = session::recorded(&mut tx, pins).await?;
            let request = match recorded {
                Some(ceiling) => runtime_authority::LiveRunSnapshotRequest::issued(pins, ceiling)?,
                None => runtime_authority::LiveRunSnapshotRequest::pre_issuance(pins),
            };
            self.invocation_source(&mut tx, &request).await?;
            tx.commit().await.map_err(storage)?;
            return Ok(recorded);
        }
        if !self.canonical_live_snapshots {
            return Err(RuntimeAuthorityError::Unsupported);
        }
        let mut tx = self.pool.begin().await.map_err(storage)?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        context::ensure_worker(&mut tx).await?;
        let exact: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE id=$1 AND instance_id=$2
                AND instance_revision_id=$3 AND release_id=$4 AND release_agent_id=$5
                AND command_id=$6 AND execution_profile='owned_canonical'
                AND vm_id=id::text AND vm_binding_kind='planned'
                AND vm_provider_namespace IS NOT NULL AND vm_host_id IS NOT NULL)",
        )
        .bind(pins.run())
        .bind(pins.instance())
        .bind(pins.revision())
        .bind(pins.release())
        .bind(pins.release_agent())
        .bind(pins.command())
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?;
        if !exact {
            return Err(RuntimeAuthorityError::Unsupported);
        }
        let recorded = session::recorded(&mut tx, pins).await?;
        tx.commit().await.map_err(storage)?;
        Ok(recorded)
    }
}
