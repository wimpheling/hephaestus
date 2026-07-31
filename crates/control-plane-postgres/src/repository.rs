//! Authorized repository read operations.

use identity_domain::AuthenticatedIdentity;
use serde_json::Value;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("repository access is denied")]
    PermissionDenied,
    #[error("repository was not found")]
    NotFound,
    #[error("repository query failed")]
    Persistence(String),
    #[error("repository page is invalid")]
    InvalidPage,
}

pub struct Page {
    pub size: i64,
    pub after: Option<Uuid>,
}

#[derive(FromRow)]
pub struct RepositoryRow {
    pub id: Uuid,
    pub name: String,
    pub default_branch: String,
    pub is_public: bool,
    pub project_id: Uuid,
    pub project_name: String,
    pub organization_id: Uuid,
    pub organization_name: String,
}

#[derive(FromRow)]
pub struct RunRow {
    pub id: Uuid,
    pub state: String,
    pub outcome: Option<String>,
    pub exit_code: Option<i32>,
    pub failure: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub agent_name: String,
    pub commit_sha: String,
    pub git_ref: String,
    pub attempt: i32,
    pub proposal_id: Option<Uuid>,
    pub proposal_state: Option<String>,
}

pub struct RepositoryResult {
    pub repository: RepositoryRow,
    pub runs: Vec<RunRow>,
}

#[derive(FromRow)]
pub struct AttachmentRow {
    pub id: Uuid,
    pub ref_selector: Value,
    pub trigger_policy: String,
    pub enabled: bool,
    pub removed_at: Option<OffsetDateTime>,
    pub instance_id: Uuid,
    pub instance_name: String,
    pub instance_state: String,
    pub project_id: Uuid,
    pub project_name: String,
    pub release_id: Uuid,
    pub release_version: String,
}

pub struct AttachmentPage {
    pub values: Vec<AttachmentRow>,
    pub next: Option<String>,
}

pub struct RepositoryApplication {
    pool: PgPool,
}

impl RepositoryApplication {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// # Errors
    ///
    /// Returns a persistence, authorization, or not-found error.
    pub async fn get(
        &self,
        identity: &AuthenticatedIdentity,
        repository_id: Uuid,
    ) -> Result<RepositoryResult, RepositoryError> {
        let mut tx = self.transaction(identity).await?;
        require_read(&mut tx, repository_id).await?;
        let repository = sqlx::query_as(
            "SELECT repository.id, repository.name, repository.default_branch,
                    repository.is_public, project.id AS project_id,
                    project.name AS project_name, organization.id AS organization_id,
                    organization.name AS organization_name
             FROM repositories repository
             JOIN projects project ON project.id = repository.project_id
             JOIN organizations organization ON organization.id = project.organization_id
             WHERE repository.id = $1",
        )
        .bind(repository_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(persistence)?
        .ok_or(RepositoryError::NotFound)?;
        let runs = sqlx::query_as(
            "SELECT run.id, run.state, run.outcome, run.exit_code, run.failure,
                    run.created_at, run.updated_at, instance.name AS agent_name,
                    request.commit_sha, request.git_ref, request.attempt,
                    proposal.id AS proposal_id, proposal.state AS proposal_state
             FROM run_requests request
             JOIN runs run ON run.id = request.run_id
             JOIN agent_instances instance ON instance.id = run.instance_id
             LEFT JOIN review_proposals proposal ON proposal.run_id = run.id
             WHERE request.repository_id = $1
             ORDER BY run.created_at DESC, run.id
             LIMIT 100",
        )
        .bind(repository_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(persistence)?;
        tx.commit().await.map_err(persistence)?;
        Ok(RepositoryResult { repository, runs })
    }

    /// # Errors
    ///
    /// Returns a persistence, authorization, or invalid-page error.
    pub async fn attachments(
        &self,
        identity: &AuthenticatedIdentity,
        repository_id: Uuid,
        page: Page,
    ) -> Result<AttachmentPage, RepositoryError> {
        let mut tx = self.transaction(identity).await?;
        require_read(&mut tx, repository_id).await?;
        let mut rows: Vec<AttachmentRow> = sqlx::query_as(
            "SELECT attachment.id, attachment.ref_selector, attachment.trigger_policy,
                    attachment.enabled, attachment.removed_at,
                    instance.id AS instance_id, instance.name AS instance_name,
                    instance.state AS instance_state, project.id AS project_id,
                    project.name AS project_name, release.id AS release_id,
                    release.version AS release_version
             FROM agent_attachments attachment
             JOIN agent_instances instance ON instance.id = attachment.instance_id
             JOIN projects project ON project.id = instance.project_id
             JOIN agent_instance_revisions revision ON revision.id = instance.active_revision_id
             JOIN release_agents release_agent ON release_agent.id = revision.release_agent_id
             JOIN releases release ON release.id = release_agent.release_id
             WHERE attachment.repository_id = $1 AND ($2::uuid IS NULL OR attachment.id > $2)
             ORDER BY attachment.id
             LIMIT $3",
        )
        .bind(repository_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(persistence)?;
        tx.commit().await.map_err(persistence)?;
        let take = usize::try_from(page.size).map_err(|_| RepositoryError::InvalidPage)?;
        let has_more = rows.len() > take;
        rows.truncate(take);
        let next = has_more
            .then(|| rows.last().map(|row| row.id.to_string()))
            .flatten();
        Ok(AttachmentPage { values: rows, next })
    }

    async fn transaction(
        &self,
        identity: &AuthenticatedIdentity,
    ) -> Result<Transaction<'_, Postgres>, RepositoryError> {
        let mut tx = self.pool.begin().await.map_err(persistence)?;
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
        .map_err(persistence)?;
        Ok(tx)
    }
}

async fn require_read(
    tx: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
) -> Result<(), RepositoryError> {
    let allowed: bool = sqlx::query_scalar(
        "SELECT check_permission('user', hephaestus_actor_id(), 'can_read',
                                 'repository', $1::text) = 1",
    )
    .bind(repository_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(persistence)?;
    if allowed {
        Ok(())
    } else {
        Err(RepositoryError::PermissionDenied)
    }
}

fn persistence(error: sqlx::Error) -> RepositoryError {
    RepositoryError::Persistence(error.to_string())
}
