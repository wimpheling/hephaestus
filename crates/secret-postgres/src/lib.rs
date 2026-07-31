//! `PostgreSQL` persistence adapter for secret runtime metadata.
#![allow(clippy::wildcard_imports)] // Adapter implementation mirrors the large provider-neutral API.
#![allow(missing_docs)] // Legacy query DTOs are replaced by typed ports incrementally.
#![allow(
    clippy::unused_async,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::struct_excessive_bools
)]
//!
//! Filesystem materialization and encrypted-store/broker effects remain in
//! `secret-runtime` and `secret-service`; this crate owns only SQL queries and
//! the journal transitions around ephemeral mounts.

mod service;

pub use service::{SecretRuntimeService, SecretService};

use identity_domain::AuthenticatedIdentity;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

/// Value-free secret metadata query failure.
#[derive(Debug, thiserror::Error)]
pub enum SecretQueryError {
    /// Query is unavailable in the current adapter.
    #[error("secret metadata query unavailable")]
    Unavailable,
    /// Invalid cursor page.
    #[error("invalid secret page")]
    InvalidPage,
    /// Database query failure.
    #[error("secret metadata persistence failed")]
    Persistence(#[source] sqlx::Error),
}
/// Cursor page request.
#[derive(Clone, Copy)]
pub struct Page {
    pub size: i64,
    pub after: Option<Uuid>,
}
/// Cursor page result.
pub struct PageResult<T> {
    pub values: Vec<T>,
    pub next_page_token: Option<String>,
}
/// Value-free secret summary.
pub struct SecretSummary {
    pub id: Uuid,
    pub name: String,
    pub status: String,
    pub allowed_delivery_modes: Vec<String>,
    pub active_version_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub active_version_sequence: Option<i64>,
    pub active_version_created_at: Option<OffsetDateTime>,
    pub grant_count: i64,
    pub import_count: i64,
    pub binding_count: i64,
    pub has_raw_binding: bool,
    pub can_rotate: bool,
    pub can_manage_grants: bool,
    pub can_revoke: bool,
    pub can_purge: bool,
}
/// Value-free grant summary.
pub struct GrantSummary {
    pub id: Uuid,
    pub secret_id: Uuid,
    pub secret_name: String,
    pub target_kind: String,
    pub target_id: Uuid,
    pub target_name: Option<String>,
    pub delivery_modes: Vec<String>,
    pub phases: Vec<String>,
    pub destinations: Vec<String>,
    pub expires_at: Option<OffsetDateTime>,
    pub status: String,
    pub created_at: OffsetDateTime,
    pub import_count: i64,
    pub import_id: Option<Uuid>,
    pub import_alias: Option<String>,
    pub import_status: Option<String>,
}
/// Value-free import summary.
pub struct ImportSummary {
    pub id: Uuid,
    pub alias: String,
    pub target_kind: String,
    pub target_id: Uuid,
    pub status: String,
    pub secret_id: Uuid,
    pub secret_name: String,
    pub secret_status: String,
    pub delivery_modes: Vec<String>,
    pub phases: Vec<String>,
    pub destinations: Vec<String>,
    pub expires_at: Option<OffsetDateTime>,
}
/// Read-only secret query facade.
pub struct SecretApplication {
    pool: PgPool,
}
impl SecretApplication {
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn list_project_secrets(
        &self,
        _: &AuthenticatedIdentity,
        _: Uuid,
        _: Page,
    ) -> Result<PageResult<SecretSummary>, SecretQueryError> {
        let _ = &self.pool;
        Err(SecretQueryError::Unavailable)
    }
    pub async fn list_organization_secrets(
        &self,
        _: &AuthenticatedIdentity,
        _: Uuid,
        _: Page,
    ) -> Result<PageResult<SecretSummary>, SecretQueryError> {
        let _ = &self.pool;
        Err(SecretQueryError::Unavailable)
    }
    pub async fn list_organization_grants(
        &self,
        _: &AuthenticatedIdentity,
        _: Uuid,
        _: Page,
    ) -> Result<PageResult<GrantSummary>, SecretQueryError> {
        let _ = &self.pool;
        Err(SecretQueryError::Unavailable)
    }
    pub async fn project_authority(
        &self,
        _: &AuthenticatedIdentity,
        _: Uuid,
        _: Page,
        _: Page,
    ) -> Result<ProjectAuthority, SecretQueryError> {
        let _ = &self.pool;
        Err(SecretQueryError::Unavailable)
    }
}
/// Project secret authority pages.
pub struct ProjectAuthority {
    pub grants: PageResult<GrantSummary>,
    pub imports: PageResult<ImportSummary>,
}

use async_trait::async_trait;
use run_domain::Run;
use runtime_types::RunId;
use secret_runtime::{
    EphemeralSecretMount, SecretDispatchInput, SecretMountManager, SecretMountMetadata,
};
use sqlx::FromRow;
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
                    COALESCE(request.actor_id, update.actor_id) AS actor_id,
                    request.request_id,
                    request.git_ref, request.commit_sha
             FROM runs AS stored_run
             JOIN agent_instance_revisions AS revision
               ON revision.id = stored_run.instance_revision_id
              AND revision.instance_id = stored_run.instance_id
             LEFT JOIN run_requests AS request ON request.run_id = stored_run.id
             LEFT JOIN agent_updates AS update
               ON update.hook_run_id = stored_run.id
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
        mount: &EphemeralSecretMount,
    ) -> Result<(), run_orchestrator::RunSecretError> {
        let directory = mount
            .host_path()
            .file_name()
            .and_then(|value| value.to_str())
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(|| redacted("ephemeral secret identity is invalid"))?;
        sqlx::query(
            "INSERT INTO secret_runtime_mounts
             (run_id, opaque_directory, state)
             VALUES ($1, $2, 'materialized')",
        )
        .bind(run_id.as_uuid())
        .bind(directory)
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
    config: secret_runtime::EphemeralSecretConfig,
) -> Result<PgSecretMountManager<D, R>, run_orchestrator::RunSecretError>
where
    D: secret_store::KeyProvider + Send + Sync,
    R: secret_store::KeyProvider + Send + Sync,
{
    SecretMountManager::initialize(
        PostgresSecretMountMetadata::new(pool),
        dispatch,
        runtime,
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
