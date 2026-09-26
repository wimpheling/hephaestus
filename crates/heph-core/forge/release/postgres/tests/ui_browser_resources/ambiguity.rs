//! Fail-closed ambiguity check for browser resource projection.

use super::{fixture, support};
use fixture::{
    fixture_session_secret, insert_authenticated_child,
    seed_fixture_reusing_installation_helpers_draft,
};
use gateway_domain::HttpMethod;
use identity_domain::RequestId;
use release_domain::{UiInstallationGenerationId, ui_browser::UiBrowserSessionSecret};
use release_postgres::PgUiBrowserServingStore;
use release_service::{UiBrowserHttpRequest, UiBrowserHttpServingProjection};
use serial_test::serial;
use std::env;
use uuid::Uuid;

#[tokio::test]
#[serial]
async fn ui_browser_resource_projection_fails_closed_on_static_api_ambiguity() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser ambiguity: test URL is unset");
        return;
    };
    let (_, worker, app) = support::connect_pools(&database_url).await;
    let fixture = seed_fixture_reusing_installation_helpers_draft(&worker).await;
    sqlx::query(
        "INSERT INTO release_ui_api_bindings
         (release_id, ui_key, api_key, gateway_name, method, route, release_agent_id)
         VALUES ($1, 'schema-ui', 'ambiguous', 'browser-service', 'GET', '/schema-ui', $2)",
    )
    .bind(fixture.release)
    .bind(fixture.release_agent)
    .execute(&worker)
    .await
    .expect("seed colliding API declaration");
    sqlx::query(
        "INSERT INTO ui_installation_bindings
         (installation_id, generation_id, binding_kind, binding_key, release_id,
          ui_key, gateway_id, gateway_revision_id, release_agent_id, gateway_name,
          method, route, exposure)
         VALUES ($1, $2, 'api', 'ambiguous', $3, 'schema-ui', $4, $5, $6,
                 'browser-service', 'GET', '/schema-ui', 'heph_authenticated')",
    )
    .bind(fixture.installation)
    .bind(fixture.generation)
    .bind(fixture.release)
    .bind(fixture.managed_gateway)
    .bind(fixture.managed_revision)
    .bind(fixture.release_agent)
    .execute(&worker)
    .await
    .expect("seed colliding installation binding");
    sqlx::query(
        "INSERT INTO gateway_routes
         (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, '/schema-ui', ARRAY['GET'])",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.managed_revision)
    .bind(fixture.managed_gateway)
    .bind(fixture.source_project)
    .execute(&worker)
    .await
    .expect("seed colliding gateway route");
    sqlx::query(
        "UPDATE releases SET state = 'published', published_at = now(),
                publication_actor_id = $2 WHERE id = $1",
    )
    .bind(fixture.release)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("publish ambiguous release");
    let secret = UiBrowserSessionSecret::from_bytes(fixture_session_secret(fixture.actor, 96));
    insert_authenticated_child(
        &worker,
        &fixture,
        secret.digest().as_bytes().to_vec(),
        "1 hour",
    )
    .await;
    let store = PgUiBrowserServingStore::new(app);
    let result = store
        .authenticate_and_project_http(
            RequestId::new(),
            secret,
            UiInstallationGenerationId::from_uuid(fixture.generation),
            UiBrowserHttpRequest::new(HttpMethod::Get, "/schema-ui").expect("ambiguous path"),
        )
        .await;
    assert!(result.is_err(), "ambiguous declarations must fail closed");
    println!("REAL_UI_BROWSER_RESOURCE_AMBIGUITY=1 exact_candidate_count=2 denied=1");
}
