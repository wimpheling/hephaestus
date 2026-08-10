//! Authorized project read operations.

use identity_domain::AuthenticatedIdentity;
use serde_json::Value;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

/// Bounded stable UUID cursor shared by project list operations.
#[derive(Clone, Copy)]
pub struct Page {
    pub size: i64,
    pub after: Option<Uuid>,
}

/// Project application failures.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("project access is denied")]
    PermissionDenied,
    #[error("project was not found")]
    NotFound,
    #[error("project query failed")]
    Persistence(#[source] sqlx::Error),
    #[error("project page is invalid")]
    InvalidPage,
}

#[derive(FromRow)]
pub struct ProjectRow {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub organization_id: Uuid,
    pub organization_name: String,
}

#[derive(FromRow)]
pub struct ProjectRepositoryRow {
    pub id: Uuid,
    pub name: String,
    pub default_branch: String,
    pub is_public: bool,
    pub attachment_count: i64,
    pub run_count: i64,
}

#[derive(FromRow)]
pub struct InstanceRow {
    pub id: Uuid,
    pub name: String,
    pub state: String,
    pub run_gate_open: bool,
    pub active_revision_id: Uuid,
    pub state_volume_id: Option<Uuid>,
    pub updated_at: OffsetDateTime,
    pub runnable: Option<bool>,
    pub platform_policy_version: Option<String>,
    pub diagnostics: Option<Value>,
    pub release_id: Option<Uuid>,
    pub release_version: Option<String>,
    pub release_state: Option<String>,
    pub release_agent_name: Option<String>,
    pub attachment_count: i64,
    pub run_count: i64,
    pub last_run_at: Option<OffsetDateTime>,
}

#[derive(FromRow)]
pub struct ReleaseAgentRow {
    pub id: Uuid,
    pub display_name: String,
    pub parameter_schema: Value,
    pub secret_slot_schema: Value,
    pub runtime_contract: Value,
    pub requires_state: bool,
    pub release_id: Uuid,
    pub release_version: String,
    pub source_commit: String,
    pub repository_id: Uuid,
    pub repository_name: String,
    pub capability_requirements: Value,
}

pub struct PageResult<T> {
    pub values: Vec<T>,
    pub next: Option<String>,
}

/// Executes project reads under transaction-local RLS identity.
pub struct ProjectApplication {
    pool: PgPool,
}

impl ProjectApplication {
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn get(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
    ) -> Result<ProjectRow, ProjectError> {
        let mut tx = self.transaction(identity).await?;
        require_permission(&mut tx, "can_read", "project", project_id).await?;
        let row = sqlx::query_as(
            "SELECT project.id, project.name,
                    COALESCE(project.settings->>'description', '') AS description,
                    organization.id AS organization_id,
                    organization.name AS organization_name
             FROM projects project
             JOIN organizations organization ON organization.id = project.organization_id
             WHERE project.id = $1",
        )
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ProjectError::Persistence)?
        .ok_or(ProjectError::NotFound)?;
        tx.commit().await.map_err(ProjectError::Persistence)?;
        Ok(row)
    }

    pub async fn repositories(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
        page: Page,
    ) -> Result<PageResult<ProjectRepositoryRow>, ProjectError> {
        let mut tx = self.transaction(identity).await?;
        require_permission(&mut tx, "can_read", "project", project_id).await?;
        let rows = sqlx::query_as(
            "SELECT repository.id, repository.name, repository.default_branch,
                    repository.is_public,
                    count(DISTINCT attachment.id)::bigint AS attachment_count,
                    count(DISTINCT request.run_id)::bigint AS run_count
             FROM repositories repository
             LEFT JOIN agent_attachments attachment ON attachment.repository_id = repository.id
                  AND attachment.removed_at IS NULL
             LEFT JOIN run_requests request ON request.repository_id = repository.id
             WHERE repository.project_id = $1 AND ($2::uuid IS NULL OR repository.id > $2)
             GROUP BY repository.id
             ORDER BY repository.id
             LIMIT $3",
        )
        .bind(project_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(ProjectError::Persistence)?;
        tx.commit().await.map_err(ProjectError::Persistence)?;
        finish_page(rows, page)
    }

    pub async fn instances(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
        page: Page,
    ) -> Result<PageResult<InstanceRow>, ProjectError> {
        let mut tx = self.transaction(identity).await?;
        require_permission(&mut tx, "can_read", "project", project_id).await?;
        let rows = sqlx::query_as(
            "SELECT instance.id, instance.name, instance.state,
                    instance.run_gate_open, instance.active_revision_id,
                    instance.state_volume_id, instance.updated_at,
                    revision.runnable, revision.platform_policy_version,
                    revision.diagnostics, release.id AS release_id,
                    release.version AS release_version, release.state AS release_state,
                    release_agent.display_name AS release_agent_name,
                    count(DISTINCT attachment.id)::bigint AS attachment_count,
                    count(DISTINCT run.id)::bigint AS run_count,
                    max(run.updated_at) AS last_run_at
             FROM agent_instances instance
             LEFT JOIN agent_instance_revisions revision ON revision.id = instance.active_revision_id
             LEFT JOIN release_agents release_agent ON release_agent.id = revision.release_agent_id
             LEFT JOIN releases release ON release.id = release_agent.release_id
             LEFT JOIN agent_attachments attachment ON attachment.instance_id = instance.id
                  AND attachment.removed_at IS NULL
             LEFT JOIN runs run ON run.instance_id = instance.id
             WHERE instance.project_id = $1 AND ($2::uuid IS NULL OR instance.id > $2)
             GROUP BY instance.id, revision.id, release.id, release_agent.id
             ORDER BY instance.id
             LIMIT $3",
        )
        .bind(project_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(ProjectError::Persistence)?;
        tx.commit().await.map_err(ProjectError::Persistence)?;
        finish_page(rows, page)
    }

    pub async fn importable_agents(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
        page: Page,
    ) -> Result<PageResult<ReleaseAgentRow>, ProjectError> {
        let mut tx = self.transaction(identity).await?;
        require_permission(&mut tx, "can_manage", "project", project_id).await?;
        let rows = sqlx::query_as(
            "SELECT release_agent.id, release_agent.display_name,
                    release_agent.parameter_schema, release_agent.secret_slot_schema,
                    release_agent.runtime_contract, release_agent.requires_state,
                    release.id AS release_id, release.version AS release_version,
                    release.source_commit, repository.id AS repository_id,
                    repository.name AS repository_name,
                    COALESCE((
                      SELECT jsonb_agg(jsonb_build_object(
                        'id', requirement.id,
                        'release_agent_id', requirement.release_agent_id,
                        'slot_key', requirement.slot_key,
                        'purpose', requirement.purpose,
                        'resource_kind', requirement.resource_kind,
                        'required_operations', requirement.required_operations,
                        'optional_operations', requirement.optional_operations,
                        'slot_required', requirement.slot_required
                      ) ORDER BY requirement.slot_key)
                      FROM release_capability_requirements AS requirement
                      WHERE requirement.release_agent_id = release_agent.id
                    ), '[]'::jsonb) AS capability_requirements
             FROM release_agents release_agent
             JOIN releases release ON release.id = release_agent.release_id
             JOIN repositories repository ON repository.id = release.repository_id
             WHERE release.state = 'published'
               AND ($1::uuid IS NULL OR release_agent.id > $1)
               AND check_permission('user', hephaestus_actor_id(), 'can_use',
                                    'release_agent', release_agent.id::text) = 1
             ORDER BY release_agent.id
             LIMIT $2",
        )
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(ProjectError::Persistence)?;
        tx.commit().await.map_err(ProjectError::Persistence)?;
        finish_page(rows, page)
    }

    async fn transaction(
        &self,
        identity: &AuthenticatedIdentity,
    ) -> Result<Transaction<'_, Postgres>, ProjectError> {
        let mut tx = self.pool.begin().await.map_err(ProjectError::Persistence)?;
        sqlx::query(
            "SELECT set_config('hephaestus.actor_id', $1, true),
                    set_config('hephaestus.subject_type', 'user', true),
                    set_config('hephaestus.request_id', $2, true),
                    set_config('hephaestus.occurrence_id', $3, true)",
        )
        .bind(identity.user_id.to_string())
        .bind(identity.request_id.to_string())
        .bind(identity.idempotency_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(ProjectError::Persistence)?;
        Ok(tx)
    }
}

async fn require_permission(
    tx: &mut Transaction<'_, Postgres>,
    permission: &str,
    kind: &str,
    id: Uuid,
) -> Result<(), ProjectError> {
    let allowed: bool = sqlx::query_scalar(
        "SELECT check_permission('user', hephaestus_actor_id(), $1, $2, $3::text) = 1",
    )
    .bind(permission)
    .bind(kind)
    .bind(id)
    .fetch_one(&mut **tx)
    .await
    .map_err(ProjectError::Persistence)?;
    if allowed {
        Ok(())
    } else {
        Err(ProjectError::PermissionDenied)
    }
}

fn finish_page<T>(mut rows: Vec<T>, page: Page) -> Result<PageResult<T>, ProjectError>
where
    T: RowId,
{
    let take = usize::try_from(page.size).map_err(|_| ProjectError::InvalidPage)?;
    let has_more = rows.len() > take;
    rows.truncate(take);
    let next = has_more.then(|| rows.last().map(RowId::id)).flatten();
    Ok(PageResult { values: rows, next })
}

trait RowId {
    fn id(&self) -> String;
}

macro_rules! row_id {
    ($($ty:ty),+ $(,)?) => {$(
        impl RowId for $ty {
            fn id(&self) -> String { self.id.to_string() }
        }
    )+};
}

row_id!(ProjectRepositoryRow, InstanceRow, ReleaseAgentRow);
