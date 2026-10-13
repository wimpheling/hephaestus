//! Read-only first-acquisition eligibility; never execution or IO authority.

use super::super::{agent_repository::PgRuntimeSessionRepository, common::storage};
use run_domain::{Run, RunKind, RunState};
use runtime_authority::RuntimeAuthorityError;

impl PgRuntimeSessionRepository {
    /// Verifies a genuine qualified Invocation before its first acquisition.
    ///
    /// Requires explicit qualified configuration, private 119 pre-acquisition
    /// SQL, and the actual worker role. `authorization_model_version` must be
    /// supplied by trusted server composition from its authorization model
    /// constant. It must never come from a request or inferred saved binding.
    /// Persisted admission, current rights, complete mount eligibility, exact
    /// Run pins and absence of all acquisition/runtime history are checked in
    /// one read-only snapshot. This returns no identity, session or credential.
    /// It cannot replace full post-acquisition authority or secret verification.
    ///
    /// # Errors
    ///
    /// Returns `Unsupported` without configuration, for another producer or
    /// phase, stale pins, withdrawn rights or any contradictory history. Storage
    /// and raw SQL role failures return the existing persistence error.
    pub async fn verify_qualified_invocation_pre_acquisition(
        &self,
        run: &Run,
        authorization_model_version: &str,
    ) -> Result<(), RuntimeAuthorityError> {
        let config = self.invocation_config_bytes()?;
        if run.kind != RunKind::Invocation
            || run.state != RunState::LeasingVolume
            || run.attachment_id.is_some()
            || run.volume_id.is_some()
            || run.lease_id.is_some()
            || run.lease_fencing_token.is_some()
            || run.cancel_requested_at.is_some()
            || run.outcome.is_some()
            || run.exit.is_some()
            || run.failure.is_some()
            || run.vm_id.is_none()
            || !(1..=128).contains(&authorization_model_version.len())
        {
            return Err(RuntimeAuthorityError::Unsupported);
        }
        let mut tx = self.pool.begin().await.map_err(storage)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let worker: bool = sqlx::query_scalar("SELECT current_user='hephaestus_worker'")
            .fetch_one(&mut *tx)
            .await
            .map_err(storage)?;
        if !worker {
            return Err(RuntimeAuthorityError::Unsupported);
        }
        let eligible: bool = sqlx::query_scalar(
            "SELECT verify_invocation119_pre_acquisition($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        )
        .bind(run.id.as_uuid())
        .bind(config)
        .bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid())
        .bind(run.release_id.as_uuid())
        .bind(run.release_agent_id.as_uuid())
        .bind(run.command_id.as_uuid())
        .bind(run.requires_state)
        .bind(run.vm_id.as_deref())
        .bind(run.state_version)
        .bind(authorization_model_version)
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?;
        if !eligible {
            return Err(RuntimeAuthorityError::Unsupported);
        }
        tx.commit().await.map_err(storage)?;
        Ok(())
    }
}
