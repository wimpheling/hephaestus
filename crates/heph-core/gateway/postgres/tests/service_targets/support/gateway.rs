//! Shared gateway, release, and service-instance fixture setup.

use super::rows::{insert_release, insert_revision};
use uuid::Uuid;

pub struct Fixture {
    pub owner: Uuid,
    pub project: Uuid,
    pub gateway: Uuid,
    pub old_service: Uuid,
    pub candidate_service: Uuid,
    pub stateless: Uuid,
    pub old_route: Uuid,
    pub old_instance: Uuid,
    pub owner_host_id: String,
}

// This fixture keeps the complete gateway/release graph in one setup helper so
// each boundary test uses the same valid persisted shape.
#[allow(clippy::too_many_lines)]
pub async fn seed_gateway(
    pool: &sqlx::PgPool,
    name: &str,
    lifecycle: &str,
    candidate_state: &str,
) -> Fixture {
    seed_gateway_with_instance_state(pool, name, lifecycle, candidate_state, "ready", false, None)
        .await
}

// This variant lets lifecycle-boundary tests start an instance in the exact
// persisted state needed to exercise the database transition trigger.
#[allow(clippy::too_many_lines)]
pub async fn seed_gateway_with_instance_state(
    pool: &sqlx::PgPool,
    name: &str,
    lifecycle: &str,
    candidate_state: &str,
    instance_state: &str,
    instance_expired: bool,
    owner_host_override: Option<&str>,
) -> Fixture {
    let owner = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let old_service = Uuid::new_v4();
    let candidate_service = Uuid::new_v4();
    let stateless = Uuid::new_v4();
    let old_route = Uuid::new_v4();

    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(owner)
        .bind(format!("target-owner-{name}"))
        .execute(pool)
        .await
        .expect("owner");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("target-org-{name}-{organization}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("target-project-{name}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository)
        .bind(project)
        .bind(format!("target-repository-{name}"))
        .execute(pool)
        .await
        .expect("repository");
    sqlx::query(
        "INSERT INTO gateways
            (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(format!("target-{name}"))
    .bind(lifecycle)
    .bind(owner)
    .execute(pool)
    .await
    .expect("gateway");

    let old_release = insert_release(pool, repository, owner, name, "published").await;
    let candidate_release = insert_release(
        pool,
        repository,
        owner,
        &format!("{name}-candidate"),
        candidate_state,
    )
    .await;
    insert_revision(
        pool,
        old_service,
        gateway,
        project,
        repository,
        owner,
        Some(old_release),
        "http.service.v1",
        [1; 32],
    )
    .await;
    insert_revision(
        pool,
        candidate_service,
        gateway,
        project,
        repository,
        owner,
        Some(candidate_release),
        "http.service.v1",
        [2; 32],
    )
    .await;
    insert_revision(
        pool, stateless, gateway, project, repository, owner, None, "http.v1", [3; 32],
    )
    .await;
    sqlx::query(
        "INSERT INTO gateway_routes
            (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, '/target', ARRAY['GET'])",
    )
    .bind(old_route)
    .bind(old_service)
    .bind(gateway)
    .bind(project)
    .execute(pool)
    .await
    .expect("service route");
    let old_instance = Uuid::new_v4();
    let owner_host_id =
        owner_host_override.map_or_else(|| format!("target-host-{name}-{gateway}"), str::to_owned);
    sqlx::query(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid,
             fencing_token, vm_id, state, lease_expires_at, heartbeat_at, cleaned_at)
         VALUES ($1, $2, $3, $7, $4, 1,
                 $5, $6,
                 CASE WHEN $8 THEN now() - interval '1 second'
                      ELSE now() + interval '10 minutes' END,
                 CASE WHEN $8 THEN now() - interval '2 seconds'
                      ELSE now() END,
                 CASE WHEN $6 = 'cleaned' THEN now() ELSE NULL END)",
    )
    .bind(old_instance)
    .bind(gateway)
    .bind(old_service)
    .bind(owner)
    .bind(format!("gateway-service-{old_instance}"))
    .bind(instance_state)
    .bind(&owner_host_id)
    .bind(instance_expired)
    .execute(pool)
    .await
    .expect("ready service instance");
    Fixture {
        owner,
        project,
        gateway,
        old_service,
        candidate_service,
        stateless,
        old_route,
        old_instance,
        owner_host_id,
    }
}
