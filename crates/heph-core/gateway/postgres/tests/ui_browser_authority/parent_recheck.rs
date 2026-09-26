//! Parent revocation recheck while service admission waits.

use super::support::{
    authority, insert_managed_authenticated_child, limits, observed_activity_pool,
    observed_worker_pool, request, seed_fixture_reusing_installation_helpers,
    wait_for_service_instance_wait,
};
use gateway_domain::{GatewayInvocationRecorder, UiGatewayAdmissionProvider, UiGatewayRequestKind};
use gateway_postgres::PostgresGatewayEdgeAuthority;
use http::Method;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::env;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn ui_gateway_rechecks_parent_after_service_instance_wait() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI gateway wait recheck: test URL is unset");
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
    let observed_worker = observed_worker_pool(&database_url).await;
    let observer = observed_activity_pool(&database_url).await;
    let fixture = seed_fixture_reusing_installation_helpers(&observed_worker).await;
    let instance_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid, fencing_token,
             vm_id, state, lease_expires_at, heartbeat_at)
         VALUES ($1, $2, $3, 'ui-gateway-wait-test', $4, 1, $5, 'ready',
                 now() + interval '10 minutes', now())",
    )
    .bind(instance_id)
    .bind(fixture.managed_gateway)
    .bind(fixture.managed_revision)
    .bind(Uuid::new_v4())
    .bind(format!("gateway-service-{instance_id}"))
    .execute(&observed_worker)
    .await
    .expect("seed ready service instance");
    let gateway = PostgresGatewayEdgeAuthority::new(observed_worker.clone(), limits());
    let child = insert_managed_authenticated_child(&observed_worker, &fixture, [95; 32]).await;
    let managed_authority = authority(
        child,
        fixture.managed_installation,
        fixture.managed_generation,
        fixture.actor,
        fixture.organization,
        "docs/wait-proof",
        UiGatewayRequestKind::Managed,
        Method::GET,
    );
    let admission = gateway
        .admit(&request(managed_authority.clone()))
        .await
        .expect("managed wait-proof route is admitted");
    let mut blocker = observed_worker
        .begin()
        .await
        .expect("begin named service lock blocker");
    sqlx::query(
        "SELECT id
           FROM gateway_service_instances
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(instance_id)
    .execute(&mut *blocker)
    .await
    .expect("lock service instance for observed blocker");
    let invocation = tokio::spawn({
        let gateway = gateway.clone();
        let route = admission.route.clone();
        let managed_authority = managed_authority.clone();
        async move {
            gateway
                .accepted_ui(&route, &managed_authority, Uuid::new_v4())
                .await
        }
    });
    wait_for_service_instance_wait(&observer).await;
    sqlx::query(
        "UPDATE human_browser_sessions
            SET revoked_at = statement_timestamp(), revocation_reason = 'logout'
          WHERE id = $1",
    )
    .bind(fixture.parent_session)
    .execute(&observed_worker)
    .await
    .expect("revoke parent while service lock is held");
    blocker
        .commit()
        .await
        .expect("release named service lock blocker");
    assert!(
        invocation
            .await
            .expect("wait-proof invocation task")
            .is_err()
    );
    let invocation_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_invocations
          WHERE gateway_id = $1 AND gateway_revision_id = $2",
    )
    .bind(fixture.managed_gateway)
    .bind(fixture.managed_revision)
    .fetch_one(&observed_worker)
    .await
    .expect("count wait-proof invocations");
    assert_eq!(
        invocation_count, 0,
        "revoked parent must produce no invocation"
    );
}
