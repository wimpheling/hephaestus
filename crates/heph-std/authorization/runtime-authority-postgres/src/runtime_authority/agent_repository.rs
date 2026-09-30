use super::common::{SessionRow, storage};
use super::gateway_helpers::{SnapshotBindingRow, stored_binding};
use capability_domain::RuntimeSessionId;
use capability_domain::{
    AuthorizationSnapshot, AuthorizationSnapshotId, WorkloadKind, WorkloadPrincipal,
};
use run_domain::Run;
use runtime_authority::RuntimeAuthorityError;
use sqlx::{PgPool, Postgres, Transaction};

/// `PostgreSQL` runtime authority repository for trusted workers.
#[derive(Clone)]
pub struct PgRuntimeSessionRepository {
    pub(super) pool: PgPool,
}

impl PgRuntimeSessionRepository {
    /// Creates a repository using a worker-role connection pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Resolves the exact immutable instance-revision binding ceiling for a
    /// dispatchable run.
    ///
    /// # Errors
    ///
    /// Fails closed for stale run lifecycle, incomplete required bindings,
    /// malformed persisted capability data, or storage failure.
    pub async fn resolve_snapshot(
        &self,
        run: &Run,
        authorization_model_version: &str,
    ) -> Result<AuthorizationSnapshot, RuntimeAuthorityError> {
        let dispatchable: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1
                FROM runs AS run
                JOIN agent_instances AS instance ON instance.id = run.instance_id
                JOIN agent_instance_revisions AS revision
                  ON revision.id = run.instance_revision_id
                 AND revision.instance_id = run.instance_id
                JOIN release_agents AS release_agent
                  ON release_agent.id = revision.release_agent_id
                JOIN releases AS release ON release.id = release_agent.release_id
                WHERE run.id = $1
                  AND run.instance_id = $2
                  AND run.instance_revision_id = $3
                  AND run.state = 'provisioning'
                  AND (
                      (run.run_kind = 'update'
                          AND instance.state = 'updating'
                          AND EXISTS (
                              SELECT 1
                              FROM agent_updates AS update
                              WHERE update.hook_run_id = run.id
                                AND update.instance_id = run.instance_id
                                AND update.candidate_revision_id = revision.id
                                AND update.state = 'hook_running'
                          ))
                      OR (run.run_kind = 'normal'
                          AND instance.state IN ('active', 'update_rejected')
                          AND instance.active_revision_id = revision.id)
                  )
                  AND revision.runnable
                  AND release.state = 'published'
                  AND NOT EXISTS (
                      SELECT 1
                      FROM release_capability_requirements AS requirement
                      WHERE requirement.release_agent_id = revision.release_agent_id
                        AND requirement.slot_required
                        AND NOT EXISTS (
                            SELECT 1 FROM agent_capability_bindings AS binding
                            WHERE binding.instance_revision_id = revision.id
                              AND binding.requirement_id = requirement.id
                        )
                  )
            )",
        )
        .bind(run.id.as_uuid())
        .bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(storage)?;
        if !dispatchable {
            return Err(RuntimeAuthorityError::Persistence);
        }

        let rows = sqlx::query_as::<_, SnapshotBindingRow>(
            "SELECT binding.id AS binding_id, requirement.id AS requirement_id,
                    requirement.slot_key, requirement.resource_kind,
                    requirement.required_operations,
                    requirement.optional_operations, requirement.slot_required,
                    requirement.normalized_hash AS requirement_hash,
                    binding.resource_id, binding.granted_operations,
                    binding.normalized_hash AS binding_hash,
                    binding.authorization_model_version
             FROM agent_capability_bindings AS binding
             JOIN release_capability_requirements AS requirement
               ON requirement.id = binding.requirement_id
              AND requirement.release_agent_id = binding.release_agent_id
             WHERE binding.instance_revision_id = $1
             ORDER BY binding.slot_key, binding.id",
        )
        .bind(run.instance_revision_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        let mut bindings = Vec::with_capacity(rows.len());
        for row in rows {
            bindings.push(stored_binding(row, authorization_model_version)?);
        }
        AuthorizationSnapshot::new(
            AuthorizationSnapshotId::from_uuid(run.id.as_uuid()),
            WorkloadPrincipal::new(
                WorkloadKind::AgentInstance,
                run.instance_id.as_uuid(),
                run.instance_revision_id.as_uuid(),
            ),
            authorization_model_version,
            bindings,
        )
        .map_err(|_| RuntimeAuthorityError::Persistence)
    }

    /// Rechecks every exact snapshotted operation against current Mélange
    /// authorization immediately before provisioning.
    ///
    /// # Errors
    ///
    /// Returns a safe persistence failure for storage errors or `false` when
    /// any operation is no longer authorized.
    pub async fn live_authorized(
        &self,
        run: &Run,
        snapshot: &AuthorizationSnapshot,
    ) -> Result<bool, RuntimeAuthorityError> {
        if snapshot.principal().id != run.instance_id.as_uuid()
            || snapshot.principal().revision_id != run.instance_revision_id.as_uuid()
        {
            return Ok(false);
        }
        for binding in snapshot.bindings() {
            for operation in binding.granted_operations() {
                let relation = format!("agent_{}", operation.as_str());
                let allowed: bool = sqlx::query_scalar(
                    "SELECT check_permission(
                        'agent_instance', $1::text, $2,
                        $3, $4::text
                    ) = 1",
                )
                .bind(run.instance_id.as_uuid())
                .bind(relation)
                .bind(binding.resource().kind.as_str())
                .bind(binding.resource().id)
                .fetch_one(&self.pool)
                .await
                .map_err(storage)?;
                if !allowed {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    pub(super) async fn locked(
        transaction: &mut Transaction<'_, Postgres>,
        session_id: RuntimeSessionId,
    ) -> Result<SessionRow, RuntimeAuthorityError> {
        sqlx::query_as::<_, SessionRow>(
            "SELECT id, snapshot_id, identity_hash, issuance_generation,
                    status, issued_at, expires_at, acknowledged_at, revoked_at
             FROM runtime_authority_sessions
             WHERE id = $1 FOR UPDATE",
        )
        .bind(session_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?
        .ok_or(RuntimeAuthorityError::NotFound)
    }
}
