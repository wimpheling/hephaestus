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

use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use sqlx::{FromRow, PgPool};
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
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
        page: Page,
    ) -> Result<PageResult<SecretSummary>, SecretQueryError> {
        validate_page(page)?;
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(SecretQueryError::Persistence)?;
        let rows = sqlx::query_as::<_, SecretRow>(
            "SELECT secret.id, secret.name, secret.status,
                    secret.allowed_delivery_modes, secret.active_version_id,
                    secret.created_at, secret.updated_at,
                    NULL::bigint AS active_version_sequence,
                    NULL::timestamptz AS active_version_created_at,
                    (SELECT count(*)::bigint FROM secret_grants AS secret_grant
                     WHERE secret_grant.secret_id = secret.id) AS grant_count,
                    (SELECT count(*)::bigint FROM secret_imports AS imported
                     WHERE imported.secret_id = secret.id) AS import_count,
                    (SELECT count(*)::bigint FROM agent_secret_bindings AS binding
                     JOIN secret_imports AS imported ON imported.id = binding.import_id
                     WHERE imported.secret_id = secret.id) AS binding_count,
                    EXISTS (SELECT 1 FROM agent_secret_bindings AS binding
                            JOIN secret_imports AS imported ON imported.id = binding.import_id
                            WHERE imported.secret_id = secret.id
                              AND binding.delivery_mode = 'raw') AS has_raw_binding,
                    check_permission('user', hephaestus_actor_id(), 'rotate',
                                     'secret', secret.id::text) = 1 AS can_rotate,
                    check_permission('user', hephaestus_actor_id(), 'manage_grants',
                                     'secret', secret.id::text) = 1 AS can_manage_grants,
                    check_permission('user', hephaestus_actor_id(), 'revoke',
                                     'secret', secret.id::text) = 1 AS can_revoke,
                    check_permission('user', hephaestus_actor_id(), 'purge',
                                     'secret', secret.id::text) = 1 AS can_purge
             FROM secrets AS secret
             WHERE secret.project_id = $1
                    AND ($2::uuid IS NULL OR (secret.name, secret.id) > (
                        SELECT cursor.name, cursor.id FROM secrets AS cursor WHERE cursor.id = $2
                    ))
                 ORDER BY secret.name, secret.id
                 LIMIT $3",
        )
        .bind(project_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(SecretQueryError::Persistence)?;
        tx.commit().await.map_err(SecretQueryError::Persistence)?;
        finish_secret_page(rows, page)
    }
    pub async fn list_organization_secrets(
        &self,
        identity: &AuthenticatedIdentity,
        organization_id: Uuid,
        page: Page,
    ) -> Result<PageResult<SecretSummary>, SecretQueryError> {
        validate_page(page)?;
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(SecretQueryError::Persistence)?;
        let rows = sqlx::query_as::<_, SecretRow>(
            "SELECT secret.id, secret.name, secret.status,
                    secret.allowed_delivery_modes, secret.active_version_id,
                    secret.created_at, secret.updated_at,
                    NULL::bigint AS active_version_sequence,
                    NULL::timestamptz AS active_version_created_at,
                    (SELECT count(*)::bigint FROM secret_grants AS secret_grant
                     WHERE secret_grant.secret_id = secret.id) AS grant_count,
                    (SELECT count(*)::bigint FROM secret_imports AS imported
                     WHERE imported.secret_id = secret.id) AS import_count,
                    (SELECT count(*)::bigint FROM agent_secret_bindings AS binding
                     JOIN secret_imports AS imported ON imported.id = binding.import_id
                     WHERE imported.secret_id = secret.id) AS binding_count,
                    EXISTS (SELECT 1 FROM agent_secret_bindings AS binding
                            JOIN secret_imports AS imported ON imported.id = binding.import_id
                            WHERE imported.secret_id = secret.id
                              AND binding.delivery_mode = 'raw') AS has_raw_binding,
                    check_permission('user', hephaestus_actor_id(), 'rotate',
                                     'secret', secret.id::text) = 1 AS can_rotate,
                    check_permission('user', hephaestus_actor_id(), 'manage_grants',
                                     'secret', secret.id::text) = 1 AS can_manage_grants,
                    check_permission('user', hephaestus_actor_id(), 'revoke',
                                     'secret', secret.id::text) = 1 AS can_revoke,
                    check_permission('user', hephaestus_actor_id(), 'purge',
                                     'secret', secret.id::text) = 1 AS can_purge
             FROM secrets AS secret
             WHERE secret.organization_id = $1
                    AND ($2::uuid IS NULL OR (secret.name, secret.id) > (
                        SELECT cursor.name, cursor.id FROM secrets AS cursor WHERE cursor.id = $2
                    ))
                 ORDER BY secret.name, secret.id
                 LIMIT $3",
        )
        .bind(organization_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(SecretQueryError::Persistence)?;
        tx.commit().await.map_err(SecretQueryError::Persistence)?;
        finish_secret_page(rows, page)
    }
    pub async fn list_organization_grants(
        &self,
        identity: &AuthenticatedIdentity,
        organization_id: Uuid,
        page: Page,
    ) -> Result<PageResult<GrantSummary>, SecretQueryError> {
        validate_page(page)?;
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(SecretQueryError::Persistence)?;
        let rows = sqlx::query_as::<_, GrantRow>(
            "SELECT secret_grant.id, secret_grant.secret_id,
                    secret.name AS secret_name, secret_grant.target_kind, secret_grant.target_id,
                    COALESCE(project.name, repository.name) AS target_name,
                    secret_grant.delivery_modes, secret_grant.phases, secret_grant.destinations,
                    secret_grant.expires_at, secret_grant.status, secret_grant.created_at,
                    (SELECT count(*)::bigint FROM secret_imports AS imported
                     WHERE imported.grant_id = secret_grant.id) AS import_count,
                    latest_import.id AS import_id, latest_import.alias AS import_alias,
                    latest_import.status AS import_status
             FROM secret_grants AS secret_grant
             JOIN secrets AS secret ON secret.id = secret_grant.secret_id
             LEFT JOIN projects AS project
                    ON secret_grant.target_kind = 'project' AND project.id = secret_grant.target_id
             LEFT JOIN repositories AS repository
                    ON secret_grant.target_kind = 'repository' AND repository.id = secret_grant.target_id
             LEFT JOIN LATERAL (
                    SELECT imported.id, imported.alias, imported.status
                    FROM secret_imports AS imported
                    WHERE imported.grant_id = secret_grant.id
                    ORDER BY imported.accepted_at DESC, imported.id DESC
                    LIMIT 1
             ) AS latest_import ON true
             WHERE secret_grant.owner_organization_id = $1
                    AND ($2::uuid IS NULL OR (secret.name, secret_grant.created_at, secret_grant.id) > (
                        SELECT cursor_secret.name, cursor.created_at, cursor.id
                        FROM secret_grants AS cursor
                        JOIN secrets AS cursor_secret ON cursor_secret.id = cursor.secret_id
                        WHERE cursor.id = $2
                    ))
                 ORDER BY secret.name, secret_grant.created_at, secret_grant.id
                 LIMIT $3",
        )
        .bind(organization_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(SecretQueryError::Persistence)?;
        tx.commit().await.map_err(SecretQueryError::Persistence)?;
        finish_grant_page(rows, page)
    }
    pub async fn project_authority(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
        grants_page: Page,
        imports_page: Page,
    ) -> Result<ProjectAuthority, SecretQueryError> {
        validate_page(grants_page)?;
        validate_page(imports_page)?;
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(SecretQueryError::Persistence)?;
        let grants = sqlx::query_as::<_, GrantRow>(
            "SELECT secret_grant.id, secret_grant.secret_id,
                    secret.name AS secret_name, secret_grant.target_kind, secret_grant.target_id,
                    COALESCE(project.name, repository.name) AS target_name,
                    secret_grant.delivery_modes, secret_grant.phases, secret_grant.destinations,
                    secret_grant.expires_at, secret_grant.status, secret_grant.created_at,
                    (SELECT count(*)::bigint FROM secret_imports AS imported
                     WHERE imported.grant_id = secret_grant.id) AS import_count,
                    latest_import.id AS import_id, latest_import.alias AS import_alias,
                    latest_import.status AS import_status
             FROM secret_grants AS secret_grant
             JOIN secrets AS secret ON secret.id = secret_grant.secret_id
             LEFT JOIN projects AS project
                    ON secret_grant.target_kind = 'project' AND project.id = secret_grant.target_id
             LEFT JOIN repositories AS repository
                    ON secret_grant.target_kind = 'repository' AND repository.id = secret_grant.target_id
             LEFT JOIN LATERAL (
                    SELECT imported.id, imported.alias, imported.status
                    FROM secret_imports AS imported
                    WHERE imported.grant_id = secret_grant.id
                    ORDER BY imported.accepted_at DESC, imported.id DESC
                    LIMIT 1
             ) AS latest_import ON true
             WHERE secret_grant.target_project_id = $1
                    AND ($2::uuid IS NULL OR (secret.name, secret_grant.created_at, secret_grant.id) > (
                        SELECT cursor_secret.name, cursor.created_at, cursor.id
                        FROM secret_grants AS cursor
                        JOIN secrets AS cursor_secret ON cursor_secret.id = cursor.secret_id
                        WHERE cursor.id = $2
                    ))
                 ORDER BY secret.name, secret_grant.created_at, secret_grant.id
                 LIMIT $3",
        )
        .bind(project_id)
        .bind(grants_page.after)
        .bind(grants_page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(SecretQueryError::Persistence)?;
        let imports = sqlx::query_as::<_, ImportRow>(
            "SELECT imported.id, imported.alias, imported.target_kind,
                    imported.target_id, imported.status, imported.secret_id,
                    secret.name AS secret_name, secret.status AS secret_status,
                    secret.allowed_delivery_modes AS delivery_modes,
                    secret_grant.phases, secret_grant.destinations, secret_grant.expires_at
             FROM secret_imports AS imported
             JOIN secret_grants AS secret_grant ON secret_grant.id = imported.grant_id
             JOIN secrets AS secret ON secret.id = imported.secret_id
             WHERE imported.target_kind = 'project' AND imported.target_id = $1
               AND ($2::uuid IS NULL OR (imported.alias, imported.id) > (
                   SELECT cursor.alias, cursor.id FROM secret_imports AS cursor
                   WHERE cursor.id = $2
               ))
             ORDER BY imported.alias, imported.id
             LIMIT $3",
        )
        .bind(project_id)
        .bind(imports_page.after)
        .bind(imports_page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(SecretQueryError::Persistence)?;
        tx.commit().await.map_err(SecretQueryError::Persistence)?;
        Ok(ProjectAuthority {
            grants: finish_grant_page(grants, grants_page)?,
            imports: finish_import_page(imports, imports_page)?,
        })
    }
}

#[derive(FromRow)]
struct SecretRow {
    id: Uuid,
    name: String,
    status: String,
    allowed_delivery_modes: Vec<String>,
    active_version_id: Option<Uuid>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    active_version_sequence: Option<i64>,
    active_version_created_at: Option<OffsetDateTime>,
    grant_count: i64,
    import_count: i64,
    binding_count: i64,
    has_raw_binding: bool,
    can_rotate: bool,
    can_manage_grants: bool,
    can_revoke: bool,
    can_purge: bool,
}

#[derive(FromRow)]
struct GrantRow {
    id: Uuid,
    secret_id: Uuid,
    secret_name: String,
    target_kind: String,
    target_id: Uuid,
    target_name: Option<String>,
    delivery_modes: Vec<String>,
    phases: Vec<String>,
    destinations: Vec<String>,
    expires_at: Option<OffsetDateTime>,
    status: String,
    created_at: OffsetDateTime,
    import_count: i64,
    import_id: Option<Uuid>,
    import_alias: Option<String>,
    import_status: Option<String>,
}

#[derive(FromRow)]
struct ImportRow {
    id: Uuid,
    alias: String,
    target_kind: String,
    target_id: Uuid,
    status: String,
    secret_id: Uuid,
    secret_name: String,
    secret_status: String,
    delivery_modes: Vec<String>,
    phases: Vec<String>,
    destinations: Vec<String>,
    expires_at: Option<OffsetDateTime>,
}

fn validate_page(page: Page) -> Result<(), SecretQueryError> {
    if (1..=100).contains(&page.size) {
        Ok(())
    } else {
        Err(SecretQueryError::InvalidPage)
    }
}

fn finish_secret_page(
    mut rows: Vec<SecretRow>,
    page: Page,
) -> Result<PageResult<SecretSummary>, SecretQueryError> {
    let take = usize::try_from(page.size).map_err(|_| SecretQueryError::InvalidPage)?;
    let has_more = rows.len() > take;
    rows.truncate(take);
    let next_page_token = has_more
        .then(|| rows.last())
        .flatten()
        .map(|row| row.id.to_string());
    Ok(PageResult {
        values: rows.into_iter().map(SecretSummary::from).collect(),
        next_page_token,
    })
}

fn finish_grant_page(
    mut rows: Vec<GrantRow>,
    page: Page,
) -> Result<PageResult<GrantSummary>, SecretQueryError> {
    let take = usize::try_from(page.size).map_err(|_| SecretQueryError::InvalidPage)?;
    let has_more = rows.len() > take;
    rows.truncate(take);
    let next_page_token = has_more
        .then(|| rows.last())
        .flatten()
        .map(|row| row.id.to_string());
    Ok(PageResult {
        values: rows.into_iter().map(GrantSummary::from).collect(),
        next_page_token,
    })
}

fn finish_import_page(
    mut rows: Vec<ImportRow>,
    page: Page,
) -> Result<PageResult<ImportSummary>, SecretQueryError> {
    let take = usize::try_from(page.size).map_err(|_| SecretQueryError::InvalidPage)?;
    let has_more = rows.len() > take;
    rows.truncate(take);
    let next_page_token = has_more
        .then(|| rows.last())
        .flatten()
        .map(|row| row.id.to_string());
    Ok(PageResult {
        values: rows.into_iter().map(ImportSummary::from).collect(),
        next_page_token,
    })
}

impl From<SecretRow> for SecretSummary {
    fn from(row: SecretRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            status: row.status,
            allowed_delivery_modes: row.allowed_delivery_modes,
            active_version_id: row.active_version_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
            active_version_sequence: row.active_version_sequence,
            active_version_created_at: row.active_version_created_at,
            grant_count: row.grant_count,
            import_count: row.import_count,
            binding_count: row.binding_count,
            has_raw_binding: row.has_raw_binding,
            can_rotate: row.can_rotate,
            can_manage_grants: row.can_manage_grants,
            can_revoke: row.can_revoke,
            can_purge: row.can_purge,
        }
    }
}

impl From<GrantRow> for GrantSummary {
    fn from(row: GrantRow) -> Self {
        Self {
            id: row.id,
            secret_id: row.secret_id,
            secret_name: row.secret_name,
            target_kind: row.target_kind,
            target_id: row.target_id,
            target_name: row.target_name,
            delivery_modes: row.delivery_modes,
            phases: row.phases,
            destinations: row.destinations,
            expires_at: row.expires_at,
            status: row.status,
            created_at: row.created_at,
            import_count: row.import_count,
            import_id: row.import_id,
            import_alias: row.import_alias,
            import_status: row.import_status,
        }
    }
}

impl From<ImportRow> for ImportSummary {
    fn from(row: ImportRow) -> Self {
        Self {
            id: row.id,
            alias: row.alias,
            target_kind: row.target_kind,
            target_id: row.target_id,
            status: row.status,
            secret_id: row.secret_id,
            secret_name: row.secret_name,
            secret_status: row.secret_status,
            delivery_modes: row.delivery_modes,
            phases: row.phases,
            destinations: row.destinations,
            expires_at: row.expires_at,
        }
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
