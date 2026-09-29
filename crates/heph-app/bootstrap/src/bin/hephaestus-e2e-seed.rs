//! Seeds the deterministic browser E2E identity and empty bare repository.
//!
//! The embedded migration directory is intentionally referenced here so a
//! newly added migration rebuilds the seed binary with the current schema.

#[path = "hephaestus-e2e-seed/catalog.rs"]
mod catalog;
#[path = "hephaestus-e2e-seed/forge.rs"]
mod forge;
#[path = "hephaestus-e2e-seed/release.rs"]
mod release;
#[path = "hephaestus-e2e-seed/roles.rs"]
mod roles;

use catalog::seed_builder_catalog;
use forge::bootstrap_forge;
use release::seed_release_catalog;
use roles::seed_secret_roles;

use forge_domain::OrganizationId;
use forge_postgres::PgForgeRepository;
use forge_service::GitStorage;
use identity_domain::UserId;
use sqlx::postgres::PgPoolOptions;
use std::{env, error::Error, path::PathBuf, sync::Arc};
use uuid::Uuid;

// Kept in the binary's compilation environment by build.rs so a newly added
// migration invalidates cached `sqlx::migrate!` output.
const MIGRATION_FINGERPRINT: &str = env!("HEPHAESTUS_MIGRATION_FINGERPRINT");

const USER_ID: &str = "10000000-0000-4000-8000-000000000001";
const OUTSIDER_USER_ID: &str = "10000000-0000-4000-8000-000000000003";
const ORGANIZATION_ID: &str = "10000000-0000-4000-8000-000000000002";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    std::hint::black_box(MIGRATION_FINGERPRINT);
    let schema_only = env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "--schema-only");
    let database_url = env::var("HEPHAESTUS_DATABASE_URL")?;
    let repository_root = PathBuf::from(env::var("HEPHAESTUS_REPOSITORY_ROOT")?);
    let artifact_root = PathBuf::from(env::var("HEPHAESTUS_ARTIFACT_ROOT")?);
    let issuer = env::var("HEPHAESTUS_BROWSER_OIDC_ISSUER")
        .unwrap_or_else(|_| String::from("http://127.0.0.1:5556"));
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;
    sqlx::migrate!("../../../migrations").run(&pool).await?;
    if schema_only {
        return Ok(());
    }

    let user_id = UserId::from_uuid(Uuid::parse_str(USER_ID)?);
    let outsider_user_id = UserId::from_uuid(Uuid::parse_str(OUTSIDER_USER_ID)?);
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
        "INSERT INTO users (id, display_name)
           VALUES ($1, 'Bea Outsider')
           ON CONFLICT (id) DO NOTHING",
    )
    .bind(outsider_user_id.as_uuid())
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO external_identities
           (user_id, issuer, subject, provider_metadata)
           VALUES ($1, $2, 'outsider', '{\"fixture\":true}'::jsonb)
           ON CONFLICT (issuer, subject) DO NOTHING",
    )
    .bind(outsider_user_id.as_uuid())
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
    let (project_id, repository) = bootstrap_forge(&pool, &forge, organization_id, user_id).await?;
    seed_builder_catalog(&pool).await?;
    seed_secret_roles(
        &pool,
        project_id.as_uuid(),
        repository.id.as_uuid(),
        user_id,
    )
    .await?;
    let release_agents =
        seed_release_catalog(&pool, repository.id.as_uuid(), user_id, &artifact_root).await?;

    println!(
        "{}",
        serde_json::json!({
            "user_id": user_id,
            "organization_id": organization_id,
            "project_id": project_id,
            "repository_id": repository.id,
            "release_agents": release_agents,
        })
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::seed_builder_catalog;
    use sqlx::postgres::PgPoolOptions;

    const FIXTURE_PUBLICATION_ID: &str = "20000000-0000-4000-8000-000000000003";

    #[tokio::test]
    async fn builder_catalog_seed_is_repeatable_after_approval() {
        let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
            eprintln!("skipping: HEPHAESTUS_POSTGRES_TEST_URL is not set");
            return;
        };
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("connect PostgreSQL");
        sqlx::migrate!("../../../migrations")
            .run(&pool)
            .await
            .expect("migrate PostgreSQL");

        seed_builder_catalog(&pool)
            .await
            .expect("initial catalog seed succeeds");
        seed_builder_catalog(&pool)
            .await
            .expect("repeated catalog seed must not rewrite immutable evidence");

        let state: String =
            sqlx::query_scalar("SELECT state FROM registry_publications WHERE id = $1::uuid")
                .bind(FIXTURE_PUBLICATION_ID)
                .fetch_one(&pool)
                .await
                .expect("fixture publication exists");
        assert_eq!(state, "approved");
    }
}
