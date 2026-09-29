//! Stateless execution-target fixture rows.

use super::support_types::Fixture;
use gateway_domain::GatewayServiceOwner;
use uuid::Uuid;

pub async fn seed_stateless(pool: &sqlx::PgPool, gateway: Uuid, project: Uuid) -> Fixture {
    let revision = Uuid::new_v4();
    let route = Uuid::new_v4();
    let invocation = Uuid::new_v4();
    let repository: Uuid = sqlx::query_scalar("SELECT repository_id FROM gateways WHERE id = $1")
        .bind(gateway)
        .fetch_one(pool)
        .await
        .expect("repository");
    let owner: Uuid = sqlx::query_scalar("SELECT created_by FROM gateways WHERE id = $1")
        .bind(gateway)
        .fetch_one(pool)
        .await
        .expect("gateway owner");
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, handler_contract,
             exposure, parameters, secret_slots, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, 'http.v1', 'public', '{}', '{}', $5, $6)",
    )
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind([8_u8; 32].as_slice())
    .bind(owner)
    .execute(pool)
    .await
    .expect("stateless revision");
    sqlx::query(
        "INSERT INTO gateway_routes
            (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, '/stateless', ARRAY['GET'])",
    )
    .bind(route)
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .execute(pool)
    .await
    .expect("stateless route");
    sqlx::query(
        "INSERT INTO gateway_invocations
            (id, gateway_id, gateway_revision_id, gateway_route_id,
             project_id, request_id, outcome)
         VALUES ($1, $2, $3, $4, $5, $6, 'accepted')",
    )
    .bind(invocation)
    .bind(gateway)
    .bind(revision)
    .bind(route)
    .bind(project)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("stateless invocation");
    Fixture {
        project,
        gateway,
        revision,
        route,
        instance: Uuid::nil(),
        invocation,
        session: Uuid::nil(),
        release: Uuid::nil(),
        owner: GatewayServiceOwner::new("execution-host", Uuid::new_v4()).expect("owner"),
        secret_lease: Uuid::nil(),
    }
}
