//! Managed and API UI admission and invocation cases.

use super::support::{
    app_pool, authority, insert_authenticated_child, insert_authenticated_child_for_installation,
    insert_managed_authenticated_child, limits, request, seed_fixture_reusing_installation_helpers,
    worker_pool,
};
use gateway_domain::{GatewayInvocationRecorder, UiGatewayAdmissionProvider, UiGatewayRequestKind};
use gateway_postgres::PostgresGatewayEdgeAuthority;
use http::Method;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::env;
use uuid::Uuid;

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn ui_gateway_worker_admits_managed_and_api_and_fails_closed() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI gateway authority: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply UI gateway migrations");
    let worker = worker_pool(&database_url).await;
    let app = app_pool(&database_url).await;
    let fixture = seed_fixture_reusing_installation_helpers(&worker).await;
    let gateway = PostgresGatewayEdgeAuthority::new(worker.clone(), limits());

    let managed_child = insert_managed_authenticated_child(&worker, &fixture, [94; 32]).await;
    let managed_authority = authority(
        managed_child,
        fixture.managed_installation,
        fixture.managed_generation,
        fixture.actor,
        fixture.organization,
        "docs/subpage",
        UiGatewayRequestKind::Managed,
        Method::GET,
    );
    let managed_admission = gateway
        .admit(&request(managed_authority.clone()))
        .await
        .expect("managed descendant is admitted");
    assert_eq!(
        managed_admission.route.gateway_revision_id,
        fixture.managed_revision
    );
    assert_eq!(
        managed_admission.gateway_path_and_query,
        "/gateway/service/subpage"
    );
    let sibling = authority(
        managed_child,
        fixture.managed_installation,
        fixture.managed_generation,
        fixture.actor,
        fixture.organization,
        "docs-other/asset.js",
        UiGatewayRequestKind::Managed,
        Method::GET,
    );
    assert!(gateway.admit(&request(sibling)).await.is_err());
    let managed_asset = authority(
        managed_child,
        fixture.managed_installation,
        fixture.managed_generation,
        fixture.actor,
        fixture.organization,
        "docs/assets/app.js",
        UiGatewayRequestKind::Managed,
        Method::GET,
    );
    let mut managed_asset_request = request(managed_asset);
    managed_asset_request.request_path_and_query.push('?');
    let managed_asset_admission = gateway
        .admit(&managed_asset_request)
        .await
        .expect("managed asset descendant is admitted");
    assert_eq!(
        managed_asset_admission.gateway_path_and_query,
        "/gateway/service/assets/app.js?"
    );

    let api_child =
        insert_authenticated_child(&worker, &fixture, [90; 32].into_iter().collect(), "1 hour")
            .await;
    let api_authority = authority(
        api_child,
        fixture.installation,
        fixture.generation,
        fixture.actor,
        fixture.organization,
        "/service/api",
        UiGatewayRequestKind::Api,
        Method::POST,
    );
    let api_admission = gateway
        .admit(&request(api_authority.clone()))
        .await
        .expect("exact API route is admitted");
    assert_eq!(api_admission.gateway_path_and_query, "/gateway/service/api");

    let relative_api = authority(
        api_child,
        fixture.installation,
        fixture.generation,
        fixture.actor,
        fixture.organization,
        "service/api",
        UiGatewayRequestKind::Api,
        Method::POST,
    );
    assert!(gateway.admit(&request(relative_api)).await.is_err());

    let mut wrong_actor = api_authority.clone();
    wrong_actor.actor_id = fixture.outsider;
    assert!(gateway.admit(&request(wrong_actor)).await.is_err());
    let mut wrong_method = api_authority.clone();
    wrong_method.method = Method::GET;
    assert!(gateway.admit(&request(wrong_method)).await.is_err());

    let before_identity_recheck: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_invocations")
            .fetch_one(&worker)
            .await
            .expect("count invocations before accepted UI identity recheck");
    let mut wrong_managed_actor = managed_authority.clone();
    wrong_managed_actor.actor_id = fixture.outsider;
    assert!(
        gateway
            .accepted_ui(
                &managed_admission.route,
                &wrong_managed_actor,
                Uuid::new_v4(),
            )
            .await
            .is_err()
    );
    let after_identity_recheck: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_invocations")
            .fetch_one(&worker)
            .await
            .expect("count invocations after accepted UI identity recheck");
    assert_eq!(
        after_identity_recheck, before_identity_recheck,
        "accepted UI identity recheck must precede insertion"
    );

    let replacement_generation = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 2, $3, 'schema-ui', 'project')",
    )
    .bind(replacement_generation)
    .bind(fixture.installation)
    .bind(fixture.release)
    .execute(&worker)
    .await
    .expect("insert stale-generation replacement");
    sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(replacement_generation)
        .execute(&worker)
        .await
        .expect("move installation generation");
    assert!(
        gateway
            .admit(&request(api_authority.clone()))
            .await
            .is_err()
    );
    sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(fixture.generation)
        .execute(&worker)
        .await
        .expect("restore installation generation");

    let unbound_child = insert_authenticated_child_for_installation(
        &worker,
        &fixture,
        fixture.other_installation,
        fixture.other_generation,
        "schema-ui-two",
        [91_u8; 32].into_iter().collect(),
    )
    .await;
    let unbound_authority = authority(
        unbound_child,
        fixture.other_installation,
        fixture.other_generation,
        fixture.actor,
        fixture.organization,
        "/service/api",
        UiGatewayRequestKind::Api,
        Method::POST,
    );
    assert!(gateway.admit(&request(unbound_authority)).await.is_err());

    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(fixture.managed_gateway)
        .execute(&worker)
        .await
        .expect("pause gateway");
    assert!(
        gateway
            .admit(&request(managed_authority.clone()))
            .await
            .is_err()
    );
    sqlx::query("UPDATE gateways SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.managed_gateway)
        .execute(&worker)
        .await
        .expect("restore gateway");

    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM gateway_invocations")
        .fetch_one(&worker)
        .await
        .expect("count invocations before service denial");
    assert!(
        gateway
            .accepted_ui(&managed_admission.route, &managed_authority, Uuid::new_v4(),)
            .await
            .is_err()
    );
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM gateway_invocations")
        .fetch_one(&worker)
        .await
        .expect("count invocations after service denial");
    assert_eq!(after, before, "service admission fails before insertion");

    let worker_verifier: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authenticate_ui_browser_session($1, $2, 'api', '/service/api', 'POST')",
    )
    .bind(vec![90_u8; 32])
    .bind(fixture.generation)
    .fetch_one(&worker)
    .await
    .expect("worker has the explicit verifier grant");
    assert_eq!(worker_verifier, 1);
    let app_verifier: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authenticate_ui_browser_session($1, $2, 'api', '/service/api', 'POST')",
    )
    .bind(vec![90_u8; 32])
    .bind(fixture.generation)
    .fetch_one(&app)
    .await
    .expect("application retains the original verifier grant");
    assert_eq!(app_verifier, 1);
    assert!(
        sqlx::query("SELECT id FROM ui_browser_sessions LIMIT 1")
            .fetch_one(&app)
            .await
            .is_err()
    );
    sqlx::query("UPDATE human_browser_sessions SET revoked_at = statement_timestamp(), revocation_reason = 'logout' WHERE id = $1")
        .bind(fixture.parent_session)
        .execute(&worker)
        .await
        .expect("revoke parent");
    assert!(gateway.admit(&request(api_authority)).await.is_err());
}
