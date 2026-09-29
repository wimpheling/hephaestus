use super::*;
use crate::fixture::FixtureSeedIds;

pub async fn seed_managed_gateway(worker: &PgPool, ids: &FixtureSeedIds) {
    sqlx::query(
        "INSERT INTO gateways
     (id, project_id, repository_id, name, lifecycle, created_by)
     VALUES ($1, $2, $3, 'browser-service', 'enabled', $4)",
    )
    .bind(ids.managed_gateway)
    .bind(ids.source_project)
    .bind(ids.repository)
    .bind(ids.actor)
    .execute(worker)
    .await
    .expect("seed browser service gateway");
    sqlx::query(
        "INSERT INTO gateway_revisions
     (id, gateway_id, project_id, repository_id, release_id,
      release_agent_id, release_agent_key, handler_contract, exposure,
      parameters, secret_slots, mailbox_slots, service_loopback_port,
      service_readiness_path, service_health_path, service_log_capture_mode,
      normalized_hash, created_by)
     VALUES ($1, $2, $3, $4, $5, $6, 'browser-service',
             'http.service.v1', 'heph_authenticated', '{}'::jsonb,
             ARRAY[]::text[], ARRAY[]::text[], 8080, '/ready', '/health',
             'disabled', $7, $8)",
    )
    .bind(ids.managed_revision)
    .bind(ids.managed_gateway)
    .bind(ids.source_project)
    .bind(ids.repository)
    .bind(ids.release)
    .bind(ids.release_agent)
    .bind(vec![5_u8; 32])
    .bind(ids.actor)
    .execute(worker)
    .await
    .expect("seed browser service revision");
    sqlx::query(
        "INSERT INTO gateway_routes
     (id, gateway_revision_id, gateway_id, project_id, path, methods)
     VALUES ($1, $2, $3, $4, '/service', ARRAY['GET', 'POST'])",
    )
    .bind(Uuid::new_v4())
    .bind(ids.managed_revision)
    .bind(ids.managed_gateway)
    .bind(ids.source_project)
    .execute(worker)
    .await
    .expect("seed browser service routes");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(ids.managed_gateway)
        .bind(ids.managed_revision)
        .execute(worker)
        .await
        .expect("activate browser service revision");
    sqlx::query(
        "UPDATE releases SET state = 'published', published_at = now(),
            publication_actor_id = $2 WHERE id = $1",
    )
    .bind(ids.release)
    .bind(ids.actor)
    .execute(worker)
    .await
    .expect("publish release after descriptors");
}
