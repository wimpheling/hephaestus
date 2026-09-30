//! Shared fixtures for service log reader integration scenarios.

use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use sqlx::postgres::PgPoolOptions;
use std::env;
use uuid::Uuid;

#[derive(Clone)]
pub struct Fixture {
    pub owner: Uuid,
    pub member: Uuid,
    pub outsider: Uuid,
    pub project: Uuid,
    pub other_project: Uuid,
    pub gateway: Uuid,
    pub revision: Uuid,
    pub instance: Uuid,
    pub owner_host_id: String,
}

pub fn identity(user: Uuid, label: &str) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        UserId::from_uuid(user),
        "reader-test",
        label,
        serde_json::json!({}),
        RequestId::new(),
    )
}

// Keep the disposable graph in one fixture so each authorization assertion
// uses the same exact project/gateway/revision/instance identity.
#[allow(clippy::too_many_lines)]
pub async fn seed_fixture(pool: &sqlx::PgPool) -> Fixture {
    let owner = Uuid::new_v4();
    let member = Uuid::new_v4();
    let outsider = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let other_project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let instance = Uuid::new_v4();
    let owner_host_id = format!("reader-host-{instance}");

    for (id, name) in [
        (owner, "reader owner"),
        (member, "reader member"),
        (outsider, "reader outsider"),
    ] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(id)
            .bind(name)
            .execute(pool)
            .await
            .expect("insert reader user");
    }
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("reader-org-{organization}"))
        .execute(pool)
        .await
        .expect("insert reader organization");
    sqlx::query("INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(organization)
        .bind(owner)
        .execute(pool)
        .await
        .expect("insert organization owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("reader-project-{project}"))
        .execute(pool)
        .await
        .expect("insert reader project");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(other_project)
        .bind(organization)
        .bind(format!("reader-other-project-{other_project}"))
        .execute(pool)
        .await
        .expect("insert reader other project");
    for user in [owner, member] {
        sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
            .bind(project)
            .bind(user)
            .execute(pool)
            .await
            .expect("insert reader project maintainer");
    }
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(other_project)
        .bind(owner)
        .execute(pool)
        .await
        .expect("insert reader other project maintainer");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository)
        .bind(project)
        .bind(format!("reader-repository-{repository}"))
        .execute(pool)
        .await
        .expect("insert reader repository");
    sqlx::query(
        "INSERT INTO gateways (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, $4, 'paused', $5)",
    )
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(format!(
        "reader-gateway-{}",
        &gateway.simple().to_string()[..8]
    ))
    .bind(owner)
    .execute(pool)
    .await
    .expect("insert reader gateway");
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, handler_contract, exposure,
             parameters, secret_slots, normalized_hash, created_by,
             service_loopback_port, service_readiness_path, service_health_path,
             service_log_capture_mode)
         VALUES ($1, $2, $3, $4, 'http.service.v1', 'public', '{}', '{}', $5, $6,
                 8080, '/readyz', '/healthz', 'application')",
    )
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind([7_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("insert reader revision");
    sqlx::query(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid, fencing_token,
             vm_id, state, lease_expires_at, heartbeat_at)
         VALUES ($1, $2, $3, $6, $4, 1, $5, 'ready',
                 now() - interval '1 minute', now() - interval '2 minutes')",
    )
    .bind(instance)
    .bind(gateway)
    .bind(revision)
    .bind(Uuid::new_v4())
    .bind(format!("gateway-service-{instance}"))
    .bind(&owner_host_id)
    .execute(pool)
    .await
    .expect("insert reader service instance");

    Fixture {
        owner,
        member,
        outsider,
        project,
        other_project,
        gateway,
        revision,
        instance,
        owner_host_id,
    }
}

pub async fn test_pool() -> Option<sqlx::PgPool> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply real PostgreSQL migrations");
    Some(pool)
}

pub async fn worker_pool() -> sqlx::PgPool {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("worker test database URL");
    PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET application_name = 'heph-service-log-project-metadata-worker'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect worker role")
}
