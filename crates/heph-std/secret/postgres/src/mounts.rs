use super::*;

use async_trait::async_trait;
use heph_secret::{
    EphemeralSecretConfig, SecretDispatchInput, SecretMountManager, SecretMountMetadata,
};
use run_domain::Run;
use runtime_types::RunId;
use std::collections::BTreeSet;
/// PostgreSQL-backed ephemeral mount metadata.
#[derive(Clone)]
pub struct PostgresSecretMountMetadata {
    pool: PgPool,
}

impl PostgresSecretMountMetadata {
    /// Creates an adapter using the caller's secret persistence pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SecretMountMetadata for PostgresSecretMountMetadata {
    async fn dispatch_input(
        &self,
        run: &Run,
    ) -> Result<Option<SecretDispatchInput>, run_orchestrator::RunSecretError> {
        return sqlx::query_as::<_, DispatchInputRow>(
            "SELECT revision.secret_bindings,
                    COALESCE(request.actor_id, update.actor_id, binding_grant.granted_by) AS actor_id,
                    COALESCE(request.request_id, invocation.request_id) AS request_id,
                    COALESCE(request.git_ref, attempt.target_ref) AS git_ref,
                    COALESCE(request.commit_sha, attempt.target_commit) AS commit_sha
             FROM runs AS stored_run
             JOIN agent_instance_revisions AS revision
               ON revision.id = stored_run.instance_revision_id
              AND revision.instance_id = stored_run.instance_id
             LEFT JOIN run_requests AS request ON request.run_id = stored_run.id
             LEFT JOIN agent_updates AS update
               ON update.hook_run_id = stored_run.id
             LEFT JOIN mailbox_delivery_attempts AS attempt ON attempt.run_id = stored_run.id
             LEFT JOIN gateway_mailbox_publications AS publication
               ON publication.event_id = attempt.event_id
              AND publication.outcome IN ('accepted', 'duplicate')
             LEFT JOIN gateway_mailbox_binding_grants AS binding_grant
               ON binding_grant.id = publication.grant_id
             LEFT JOIN gateway_invocations AS invocation
               ON invocation.id = publication.invocation_id
             WHERE stored_run.id = $1
               AND stored_run.instance_id = $2
               AND stored_run.instance_revision_id = $3",
        )
        .bind(run.id.as_uuid())
        .bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)
        .map(|row| {
            row.map(|row| SecretDispatchInput {
                secret_bindings: row.secret_bindings,
                actor_id: row.actor_id,
                request_id: row.request_id,
                git_ref: row.git_ref,
                commit_sha: row.commit_sha,
            })
        });
    }

    async fn persist_mount(
        &self,
        run_id: RunId,
        opaque_directory: Uuid,
    ) -> Result<(), run_orchestrator::RunSecretError> {
        sqlx::query(
            "INSERT INTO secret_runtime_mounts
             (run_id, opaque_directory, state)
             VALUES ($1, $2, 'materialized')",
        )
        .bind(run_id.as_uuid())
        .bind(opaque_directory)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn authorized(&self, run: &Run) -> Result<bool, run_orchestrator::RunSecretError> {
        sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM secret_runtime_sessions AS session
                WHERE session.run_id = $1 AND session.instance_id = $2
                  AND session.instance_revision_id = $3
                  AND session.status = 'active' AND session.expires_at > now()
                  AND (SELECT count(*) FROM run_secret_provenance
                       WHERE run_id = session.run_id) =
                      (SELECT count(*) FROM secret_leases AS lease
                       JOIN agent_secret_bindings AS binding
                         ON binding.id = lease.binding_id AND binding.status = 'active'
                       JOIN secret_imports AS imported
                         ON imported.id = binding.import_id AND imported.status = 'active'
                       JOIN secret_grants AS source_grant
                         ON source_grant.id = imported.grant_id AND source_grant.status = 'active'
                        AND (source_grant.expires_at IS NULL OR source_grant.expires_at > now())
                       JOIN secrets AS secret ON secret.id = imported.secret_id AND secret.status = 'active'
                       JOIN secret_versions AS version
                         ON version.id = lease.secret_version_id AND version.secret_id = secret.id
                        AND version.status = 'active'
                       WHERE lease.session_id = session.id AND lease.run_id = session.run_id
                         AND lease.status = 'active' AND lease.expires_at > now()))",
        )
        .bind(run.id.as_uuid())
        .bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(db_error)
    }

    async fn materialized_directory(
        &self,
        run_id: RunId,
    ) -> Result<Option<Uuid>, run_orchestrator::RunSecretError> {
        sqlx::query_scalar(
            "SELECT opaque_directory FROM secret_runtime_mounts
             WHERE run_id = $1 AND state = 'materialized'",
        )
        .bind(run_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)
    }

    async fn mark_destroyed(&self, run_id: RunId) -> Result<(), run_orchestrator::RunSecretError> {
        sqlx::query(
            "UPDATE secret_runtime_mounts SET state = 'destroyed', destroyed_at = now()
             WHERE run_id = $1 AND state = 'materialized'",
        )
        .bind(run_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn live_directories(&self) -> Result<BTreeSet<String>, run_orchestrator::RunSecretError> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT mount.opaque_directory FROM secret_runtime_mounts AS mount
             JOIN runs AS run ON run.id = mount.run_id
             WHERE mount.state = 'materialized' AND run.state <> 'cleaned_up'",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(rows
            .into_iter()
            .map(|value| value.simple().to_string())
            .collect())
    }

    async fn mark_cleaned_mounts_destroyed(&self) -> Result<(), run_orchestrator::RunSecretError> {
        sqlx::query(
            "UPDATE secret_runtime_mounts AS mount
             SET state = 'destroyed', destroyed_at = now()
             FROM runs AS run
             WHERE run.id = mount.run_id AND mount.state = 'materialized'
               AND run.state = 'cleaned_up'",
        )
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }
}

/// PostgreSQL-backed run secret manager.
pub type PgSecretMountManager<D, R> =
    SecretMountManager<PostgresSecretMountMetadata, SecretService<D>, SecretRuntimeService<R>>;

/// Constructs a PostgreSQL-backed run secret manager.
///
/// # Errors
///
/// Returns a redacted initialization error when the ephemeral root is unsafe.
pub fn initialize_manager<D, R>(
    pool: PgPool,
    dispatch: SecretService<D>,
    runtime: SecretRuntimeService<R>,
    config: EphemeralSecretConfig,
) -> Result<PgSecretMountManager<D, R>, run_orchestrator::RunSecretError>
where
    D: secret_store::KeyProvider + Send + Sync,
    R: secret_store::KeyProvider + Send + Sync,
{
    let provider = runtime
        .mount_provider()
        .ok_or_else(|| redacted("secret filesystem provider is unavailable"))?;
    SecretMountManager::initialize(
        PostgresSecretMountMetadata::new(pool),
        dispatch,
        runtime,
        provider,
        config,
    )
}

#[derive(Debug, FromRow)]
struct DispatchInputRow {
    secret_bindings: serde_json::Value,
    actor_id: Option<Uuid>,
    request_id: Option<Uuid>,
    git_ref: Option<String>,
    commit_sha: Option<String>,
}

fn db_error(_: sqlx::Error) -> run_orchestrator::RunSecretError {
    redacted("secret runtime persistence failed")
}

fn redacted(message: &str) -> run_orchestrator::RunSecretError {
    run_orchestrator::RunSecretError::redacted(message)
}
