use identity_domain::AuthenticatedIdentity;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use super::helpers::truncate_page;
use super::types::{
    Organization, OrganizationError, OrganizationPage, OrganizationPageResult, OrganizationSummary,
    ProjectPageResult, ProjectSummary, RepositoryPageResult, RepositorySummary,
};

/// Executes organization reads with transaction-local RLS identity.
pub struct OrganizationApplication {
    pool: PgPool,
}

impl OrganizationApplication {
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn list_organizations(
        &self,
        identity: &AuthenticatedIdentity,
        page: OrganizationPage,
    ) -> Result<OrganizationPageResult, OrganizationError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(OrganizationError::Persistence)?;
        sqlx::query(
            "SELECT set_config('hephaestus.actor_id', $1, true),
                    set_config('hephaestus.subject_type', 'user', true),
                    set_config('hephaestus.request_id', $2, true),
                    set_config('hephaestus.occurrence_id', $3, true)",
        )
        .bind(identity.user_id.to_string())
        .bind(identity.request_id.to_string())
        .bind(identity.idempotency_id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(OrganizationError::Persistence)?;
        let rows: Vec<(Uuid, String, i64, i64)> = sqlx::query_as(
            "SELECT organization.id, organization.name,
                    count(DISTINCT project.id)::bigint AS project_count,
                    count(DISTINCT repository.id)::bigint AS repository_count
             FROM organizations organization
             LEFT JOIN projects project ON project.organization_id = organization.id
             LEFT JOIN repositories repository ON repository.project_id = project.id
             WHERE $1::uuid IS NULL
                OR (organization.name, organization.id) > (
                    SELECT cursor.name, cursor.id
                    FROM organizations cursor
                    WHERE cursor.id = $1
                )
             GROUP BY organization.id, organization.name
             ORDER BY organization.name, organization.id
             LIMIT $2",
        )
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *transaction)
        .await
        .map_err(OrganizationError::Persistence)?;
        transaction
            .commit()
            .await
            .map_err(OrganizationError::Persistence)?;

        let has_more = i64::try_from(rows.len()).is_ok_and(|length| length > page.size);
        let take = usize::try_from(page.size).map_err(|_| OrganizationError::InvalidPage)?;
        let organizations = rows
            .into_iter()
            .take(take)
            .map(
                |(id, name, project_count, repository_count)| OrganizationSummary {
                    id,
                    name,
                    project_count,
                    repository_count,
                },
            )
            .collect::<Vec<_>>();
        let next_page_token = has_more
            .then(|| organizations.last())
            .flatten()
            .map(|last| last.id.to_string());
        Ok(OrganizationPageResult {
            organizations,
            next_page_token,
        })
    }

    pub async fn get_organization(
        &self,
        identity: &AuthenticatedIdentity,
        organization_id: Uuid,
    ) -> Result<Organization, OrganizationError> {
        let mut transaction = self.transaction(identity).await?;
        let organization =
            sqlx::query_as::<_, (Uuid, String)>("SELECT id, name FROM organizations WHERE id = $1")
                .bind(organization_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(OrganizationError::Persistence)?
                .map(|(id, name)| Organization { id, name })
                .ok_or(OrganizationError::NotFound)?;
        transaction
            .commit()
            .await
            .map_err(OrganizationError::Persistence)?;
        Ok(organization)
    }

    pub async fn list_repositories(
        &self,
        identity: &AuthenticatedIdentity,
        organization_id: Uuid,
        page: OrganizationPage,
    ) -> Result<RepositoryPageResult, OrganizationError> {
        let mut transaction = self.transaction(identity).await?;
        let rows = sqlx::query_as::<
            _,
            (
                Uuid,
                String,
                String,
                bool,
                String,
                i64,
                Option<OffsetDateTime>,
            ),
        >(
            "SELECT repository.id, repository.name, repository.default_branch,
                    repository.is_public, project.name AS project_name,
                    count(run.id)::bigint AS run_count,
                    max(run.created_at) AS last_run_at
             FROM repositories repository
             JOIN projects project ON project.id = repository.project_id
             LEFT JOIN run_requests request ON request.repository_id = repository.id
             LEFT JOIN runs run ON run.id = request.run_id
             WHERE project.organization_id = $1
               AND ($2::uuid IS NULL OR (project.name, repository.name, repository.id) > (
                    SELECT cursor_project.name, cursor.name, cursor.id
                    FROM repositories cursor
                    JOIN projects cursor_project ON cursor_project.id = cursor.project_id
                    WHERE cursor.id = $2))
             GROUP BY repository.id, repository.name, repository.default_branch,
                      repository.is_public, project.name
             ORDER BY project.name, repository.name, repository.id
             LIMIT $3",
        )
        .bind(organization_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *transaction)
        .await
        .map_err(OrganizationError::Persistence)?;
        transaction
            .commit()
            .await
            .map_err(OrganizationError::Persistence)?;
        let (rows, next_page_token) = truncate_page(rows, page.size, |row| row.0)?;
        Ok(RepositoryPageResult {
            repositories: rows
                .into_iter()
                .map(
                    |(
                        id,
                        name,
                        default_branch,
                        is_public,
                        project_name,
                        run_count,
                        last_run_at,
                    )| {
                        RepositorySummary {
                            id,
                            name,
                            default_branch,
                            is_public,
                            project_name,
                            run_count,
                            last_run_at,
                        }
                    },
                )
                .collect(),
            next_page_token,
        })
    }

    pub async fn list_projects(
        &self,
        identity: &AuthenticatedIdentity,
        organization_id: Uuid,
        page: OrganizationPage,
    ) -> Result<ProjectPageResult, OrganizationError> {
        let mut transaction = self.transaction(identity).await?;
        let rows = sqlx::query_as::<_, (Uuid, String, i64, i64, i64, Option<OffsetDateTime>)>(
            "SELECT project.id, project.name,
                    count(DISTINCT repository.id)::bigint AS repository_count,
                    count(DISTINCT instance.id)::bigint AS instance_count,
                    count(DISTINCT run.id)::bigint AS run_count,
                    max(run.updated_at) AS last_activity_at
             FROM projects project
             LEFT JOIN repositories repository ON repository.project_id = project.id
             LEFT JOIN agent_instances instance ON instance.project_id = project.id
             LEFT JOIN runs run ON run.instance_id = instance.id
             WHERE project.organization_id = $1
               AND ($2::uuid IS NULL OR (project.name, project.id) > (
                    SELECT cursor.name, cursor.id FROM projects cursor WHERE cursor.id = $2))
             GROUP BY project.id, project.name
             ORDER BY project.name, project.id
             LIMIT $3",
        )
        .bind(organization_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *transaction)
        .await
        .map_err(OrganizationError::Persistence)?;
        transaction
            .commit()
            .await
            .map_err(OrganizationError::Persistence)?;
        let (rows, next_page_token) = truncate_page(rows, page.size, |row| row.0)?;
        Ok(ProjectPageResult {
            projects: rows
                .into_iter()
                .map(
                    |(id, name, repository_count, instance_count, run_count, last_activity_at)| {
                        ProjectSummary {
                            id,
                            name,
                            repository_count,
                            instance_count,
                            run_count,
                            last_activity_at,
                        }
                    },
                )
                .collect(),
            next_page_token,
        })
    }

    async fn transaction<'a>(
        &'a self,
        identity: &AuthenticatedIdentity,
    ) -> Result<sqlx::Transaction<'a, sqlx::Postgres>, OrganizationError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(OrganizationError::Persistence)?;
        sqlx::query(
            "SELECT set_config('hephaestus.actor_id', $1, true),
                    set_config('hephaestus.subject_type', 'user', true),
                    set_config('hephaestus.request_id', $2, true),
                    set_config('hephaestus.occurrence_id', $3, true)",
        )
        .bind(identity.user_id.to_string())
        .bind(identity.request_id.to_string())
        .bind(identity.idempotency_id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(OrganizationError::Persistence)?;
        Ok(transaction)
    }
}
