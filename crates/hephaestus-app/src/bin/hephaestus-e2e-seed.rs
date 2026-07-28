//! Seeds the deterministic browser E2E identity and empty bare repository.

use forge_domain::{GitRef, OrganizationId};
use forge_service::{CreateRepository, GitStorage, PgForgeRepository};
use identity_domain::UserId;
use sqlx::postgres::PgPoolOptions;
use std::{env, error::Error, path::PathBuf, sync::Arc};
use uuid::Uuid;

const USER_ID: &str = "10000000-0000-4000-8000-000000000001";
const ORGANIZATION_ID: &str = "10000000-0000-4000-8000-000000000002";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let database_url = env::var("HEPHAESTUS_DATABASE_URL")?;
    let repository_root = PathBuf::from(env::var("HEPHAESTUS_REPOSITORY_ROOT")?);
    let issuer = env::var("HEPHAESTUS_BROWSER_OIDC_ISSUER")
        .unwrap_or_else(|_| String::from("http://127.0.0.1:5556"));
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    configure_browser_role(&pool).await?;

    let user_id = UserId::from_uuid(Uuid::parse_str(USER_ID)?);
    let organization_id = OrganizationId::from_uuid(Uuid::parse_str(ORGANIZATION_ID)?);
    sqlx::query(
        "INSERT INTO users (id, display_name)
         VALUES ($1, 'Ada Reviewer')
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(user_id.as_uuid())
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO external_identities
         (user_id, issuer, subject, provider_metadata)
         VALUES ($1, $2, 'reviewer', '{\"fixture\":true}'::jsonb)
         ON CONFLICT (issuer, subject) DO NOTHING",
    )
    .bind(user_id.as_uuid())
    .bind(&issuer)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO organizations (id, name)
         VALUES ($1, 'Acme Research')
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(organization_id.as_uuid())
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')
         ON CONFLICT (organization_id, user_id) DO UPDATE SET role = 'owner'",
    )
    .bind(organization_id.as_uuid())
    .bind(user_id.as_uuid())
    .execute(&pool)
    .await?;

    let storage = Arc::new(GitStorage::initialize(&repository_root).await?);
    let forge = PgForgeRepository::new(pool.clone(), Arc::clone(&storage));
    let project = forge
        .create_project_trusted(organization_id, "autonomy-lab")
        .await?;
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(project.id.as_uuid())
    .bind(user_id.as_uuid())
    .execute(&pool)
    .await?;
    let repository = forge
        .create_repository_trusted(&CreateRepository {
            project_id: project.id,
            name: String::from("agent-workbench"),
            default_branch: GitRef::parse("refs/heads/main")?,
            is_public: false,
            agent_runs_enabled: true,
        })
        .await?;

    println!(
        "{}",
        serde_json::json!({
            "user_id": user_id,
            "organization_id": organization_id,
            "project_id": project.id,
            "repository_id": repository.id,
        })
    );
    Ok(())
}

async fn configure_browser_role(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DO $$
         BEGIN
             IF NOT EXISTS (
                 SELECT 1 FROM pg_roles WHERE rolname = 'hephaestus_web_e2e'
             ) THEN
                 CREATE ROLE hephaestus_web_e2e
                     LOGIN PASSWORD 'hephaestus-web-e2e'
                     NOBYPASSRLS IN ROLE hephaestus_app;
             ELSE
                 ALTER ROLE hephaestus_web_e2e
                     LOGIN PASSWORD 'hephaestus-web-e2e' NOBYPASSRLS;
                 GRANT hephaestus_app TO hephaestus_web_e2e;
             END IF;
         END
         $$",
    )
    .execute(pool)
    .await?;
    Ok(())
}
