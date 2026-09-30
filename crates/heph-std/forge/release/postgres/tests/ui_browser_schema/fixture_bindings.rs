use super::*;
use crate::fixture::FixtureSeedIds;

pub async fn seed_installation_bindings(worker: &PgPool, ids: &FixtureSeedIds) {
    sqlx::query(
        "INSERT INTO ui_installation_bindings
     (installation_id, generation_id, binding_kind, binding_key,
      release_id, ui_key, gateway_id, gateway_revision_id,
      release_agent_id, gateway_name, method, route, exposure)
     VALUES
        ($1, $2, 'api', 'status', $3, 'schema-ui', $4, $5, $6,
         'browser-service', 'POST', '/service/api', 'heph_authenticated'),
        ($7, $8, 'managed_service', 'service', $3, 'schema-managed',
         $4, $5, $6, 'browser-service', 'GET', '/service',
         'heph_authenticated'),
        ($7, $8, 'api', 'status', $3, 'schema-managed', $4, $5, $6,
         'browser-service', 'POST', '/service/api', 'heph_authenticated')",
    )
    .bind(ids.installation)
    .bind(ids.generation)
    .bind(ids.release)
    .bind(ids.managed_gateway)
    .bind(ids.managed_revision)
    .bind(ids.release_agent)
    .bind(ids.managed_installation)
    .bind(ids.managed_generation)
    .execute(worker)
    .await
    .expect("seed static and managed/API installation bindings");
}
