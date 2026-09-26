use forge_domain::{GitRef, OrganizationId, ProjectId, Repository, RepositoryId};
use forge_postgres::PgForgeRepository;
use forge_service::CreateRepository;
use identity_domain::UserId;
use std::error::Error;
use uuid::Uuid;

pub async fn bootstrap_forge(
    pool: &sqlx::PgPool,
    forge: &PgForgeRepository,
    organization_id: OrganizationId,
    user_id: UserId,
) -> Result<(ProjectId, Repository), Box<dyn Error>> {
    let project_id = match sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM projects WHERE organization_id = $1 AND name = 'autonomy-lab'",
    )
    .bind(organization_id.as_uuid())
    .fetch_optional(pool)
    .await?
    {
        Some(id) => ProjectId::from_uuid(id),
        None => {
            forge
                .create_project_trusted(organization_id, "autonomy-lab")
                .await?
                .id
        }
    };
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
           VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(project_id.as_uuid())
    .bind(user_id.as_uuid())
    .execute(pool)
    .await?;
    let repository = match sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM repositories WHERE project_id = $1 AND name = 'agent-workbench'",
    )
    .bind(project_id.as_uuid())
    .fetch_optional(pool)
    .await?
    {
        Some(id) => forge.get_repository(RepositoryId::from_uuid(id)).await?,
        None => {
            forge
                .create_repository_trusted(&CreateRepository {
                    project_id,
                    name: String::from("agent-workbench"),
                    default_branch: GitRef::parse("refs/heads/main")?,
                    is_public: false,
                    agent_runs_enabled: true,
                })
                .await?
        }
    };
    Ok((project_id, repository))
}
