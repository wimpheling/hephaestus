use super::Fixture;
use forge_domain::ProjectId;
use forge_domain::RepositoryId;
use identity_domain::OrganizationId;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use secret_domain::SecretCommandKey;
use serde_json::json;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

pub async fn pool() -> Option<PgPool> {
    let url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    Some(
        PgPoolOptions::new()
            .max_connections(8)
            .connect(&url)
            .await
            .expect("connect PostgreSQL"),
    )
}

pub async fn role_pool(role: &'static str) -> PgPool {
    let url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL")
        .expect("role pool requires HEPHAESTUS_POSTGRES_TEST_URL");
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(move |connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SELECT set_config('role', $1, false)")
                    .bind(role)
                    .execute(connection)
                    .await
                    .map(|_| ())
            })
        })
        .connect(&url)
        .await
        .expect("connect role-specific PostgreSQL pool")
}

pub fn identity(user_id: UserId) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        user_id,
        "https://issuer.secret.test",
        format!("secret-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}

pub fn key(operation: &str, id: Uuid) -> SecretCommandKey {
    SecretCommandKey::derive(operation, &[id.as_bytes()])
}

pub async fn seed(pool: &PgPool) -> Fixture {
    let fixture = Fixture {
        owner: UserId::new(),
        organization_secret_manager: UserId::new(),
        ordinary_member: UserId::new(),
        target_manager: UserId::new(),
        other_owner: UserId::new(),
        organization: OrganizationId::new(),
        target_project: ProjectId::new(),
        target_repository: RepositoryId::new(),
        other_project: ProjectId::new(),
    };
    let other_organization = OrganizationId::new();
    seed_users(pool, &fixture).await;
    sqlx::query(
        "INSERT INTO organizations (id, name)
          VALUES ($1, $3), ($2, $4)",
    )
    .bind(fixture.organization.as_uuid())
    .bind(other_organization.as_uuid())
    .bind(format!("secret-org-{}", fixture.organization))
    .bind(format!("other-org-{other_organization}"))
    .execute(pool)
    .await
    .expect("seed organizations");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
          VALUES ($1, $2, 'owner'), ($1, $3, 'member'), ($1, $4, 'member'),
                 ($5, $6, 'owner')",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.owner.as_uuid())
    .bind(fixture.organization_secret_manager.as_uuid())
    .bind(fixture.ordinary_member.as_uuid())
    .bind(other_organization.as_uuid())
    .bind(fixture.other_owner.as_uuid())
    .execute(pool)
    .await
    .expect("seed owners");
    sqlx::query(
        "INSERT INTO organization_secret_managers (organization_id, user_id)
          VALUES ($1, $2)",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.organization_secret_manager.as_uuid())
    .execute(pool)
    .await
    .expect("seed organization secret manager");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name)
          VALUES ($1, $2, $3), ($4, $5, $6)",
    )
    .bind(fixture.target_project.as_uuid())
    .bind(fixture.organization.as_uuid())
    .bind(format!("target-{}", fixture.target_project))
    .bind(fixture.other_project.as_uuid())
    .bind(other_organization.as_uuid())
    .bind(format!("other-{}", fixture.other_project))
    .execute(pool)
    .await
    .expect("seed projects");
    sqlx::query(
        "INSERT INTO project_secret_roles (project_id, user_id, role)
          VALUES ($1, $2, 'secret_manager')",
    )
    .bind(fixture.target_project.as_uuid())
    .bind(fixture.target_manager.as_uuid())
    .execute(pool)
    .await
    .expect("seed target manager");
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
          VALUES ($1, $2), ($1, $3)",
    )
    .bind(fixture.target_project.as_uuid())
    .bind(fixture.target_manager.as_uuid())
    .bind(fixture.ordinary_member.as_uuid())
    .execute(pool)
    .await
    .expect("seed instance manager");
    sqlx::query(
        "INSERT INTO repositories
          (id, project_id, name, default_branch, is_public)
          VALUES ($1, $2, $3, 'refs/heads/main', false)",
    )
    .bind(fixture.target_repository.as_uuid())
    .bind(fixture.target_project.as_uuid())
    .bind(format!("target-{}", fixture.target_repository))
    .execute(pool)
    .await
    .expect("seed target repository");
    fixture
}

pub async fn seed_users(pool: &PgPool, fixture: &Fixture) {
    for (user, name) in [
        (fixture.owner, "secret-owner"),
        (
            fixture.organization_secret_manager,
            "organization-secret-manager",
        ),
        (fixture.ordinary_member, "ordinary-member"),
        (fixture.target_manager, "target-secret-manager"),
        (fixture.other_owner, "other-owner"),
    ] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(user.as_uuid())
            .bind(format!("{name}-{user}"))
            .execute(pool)
            .await
            .expect("seed user");
    }
}
