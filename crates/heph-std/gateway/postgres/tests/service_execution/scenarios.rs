//! Execution-target resolution and rejection scenarios.

use super::support_database::{test_pool, worker_pool};
use super::support_seed::{seed_service, seed_service_inner, seed_service_with_secret_lease};
use super::support_stateless::seed_stateless;
use super::support_types::{TimingWindow, assert_unavailable};
use gateway_domain::{
    GatewayExecutionTarget, GatewayExecutionTargetError, GatewayExecutionTargetResolver,
    GatewayServiceOwner,
};
use gateway_postgres::PostgresGatewayExecutionTargetResolver;
use serial_test::serial;
use std::time::Duration;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn execution_target_resolves_service_and_stateless_targets() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_service(&pool).await;
    let resolver = PostgresGatewayExecutionTargetResolver::new(worker_pool().await);
    let target = resolver
        .resolve_execution_target(
            fixture.invocation,
            fixture.route,
            fixture.revision,
            &fixture.owner,
        )
        .await
        .expect("service target");
    let GatewayExecutionTarget::Service(budget) = target else {
        panic!("service invocation resolved as stateless");
    };
    assert_eq!(budget.instance.identity.instance_id, fixture.instance);
    assert_eq!(budget.instance.identity.gateway_id, fixture.gateway);
    assert_eq!(budget.instance.identity.revision_id, fixture.revision);
    assert_eq!(budget.instance.fencing_token, 1);
    assert!(budget.remaining > Duration::from_secs(1));
    assert!(budget.remaining <= Duration::from_secs(600));

    let stateless = seed_stateless(&pool, fixture.gateway, fixture.project).await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway)
        .bind(stateless.revision)
        .execute(&pool)
        .await
        .expect("cut over gateway active pointer");
    sqlx::query("UPDATE gateway_service_instances SET state = 'draining' WHERE id = $1")
        .bind(fixture.instance)
        .execute(&pool)
        .await
        .expect("drain accepted service instance");
    let draining = resolver
        .resolve_execution_target(
            fixture.invocation,
            fixture.route,
            fixture.revision,
            &fixture.owner,
        )
        .await
        .expect("draining old target remains executable");
    assert!(matches!(draining, GatewayExecutionTarget::Service(_)));

    let target = resolver
        .resolve_execution_target(
            stateless.invocation,
            stateless.route,
            stateless.revision,
            &fixture.owner,
        )
        .await
        .expect("stateless target");
    assert_eq!(target, GatewayExecutionTarget::Stateless);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn execution_target_rejects_wrong_binding_and_exact_identity() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_service(&pool).await;
    let resolver = PostgresGatewayExecutionTargetResolver::new(worker_pool().await);
    let wrong_host = GatewayServiceOwner::new("other-host", fixture.owner.owner_uuid)
        .expect("same daemon on wrong host");
    let wrong_daemon = GatewayServiceOwner::new(fixture.owner.host_id.clone(), Uuid::new_v4())
        .expect("different daemon on same host");
    for (route, revision, owner) in [
        (fixture.route, fixture.revision, wrong_host),
        (fixture.route, fixture.revision, wrong_daemon),
        (Uuid::new_v4(), fixture.revision, fixture.owner.clone()),
        (fixture.route, Uuid::new_v4(), fixture.owner.clone()),
    ] {
        assert_eq!(
            resolver
                .resolve_execution_target(fixture.invocation, route, revision, &owner)
                .await,
            Err(GatewayExecutionTargetError::Unavailable)
        );
    }
    sqlx::query(
        "UPDATE gateway_invocations
            SET service_instance_fencing_token = 2
          WHERE id = $1",
    )
    .bind(fixture.invocation)
    .execute(&pool)
    .await
    .expect_err("immutable binding must reject a fence mutation");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn execution_target_rejects_terminal_session_expiry_and_release_revocation() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let resolver = PostgresGatewayExecutionTargetResolver::new(worker_pool().await);

    let expired = seed_service(&pool).await;
    sqlx::query(
        "UPDATE gateway_runtime_authority_sessions
            SET status = 'expired'
          WHERE id = $1",
    )
    .bind(expired.session)
    .execute(&pool)
    .await
    .expect("expire host session");
    assert_unavailable(&resolver, &expired).await;

    let revoked = seed_service(&pool).await;
    sqlx::query(
        "UPDATE gateway_runtime_authority_sessions
            SET status = 'revoked', revoked_at = now(), revocation_reason = 'test'
          WHERE id = $1",
    )
    .bind(revoked.session)
    .execute(&pool)
    .await
    .expect("revoke host session");
    assert_unavailable(&resolver, &revoked).await;

    let terminal = seed_service(&pool).await;
    sqlx::query_scalar::<_, bool>("SELECT gateway_invocation_complete($1, 'timed_out')")
        .bind(terminal.invocation)
        .fetch_one(&pool)
        .await
        .expect("complete invocation");
    assert_unavailable(&resolver, &terminal).await;

    let release = seed_service(&pool).await;
    sqlx::query("UPDATE releases SET state = 'revoked', revoked_at = now() WHERE id = $1")
        .bind(release.release)
        .execute(&pool)
        .await
        .expect("revoke release");
    assert_unavailable(&resolver, &release).await;

    let expired_session = seed_service_inner(
        &pool,
        false,
        TimingWindow::Expired,
        TimingWindow::Live,
        TimingWindow::Live,
    )
    .await;
    assert_unavailable(&resolver, &expired_session).await;

    let expired_instance = seed_service_inner(
        &pool,
        false,
        TimingWindow::Live,
        TimingWindow::Expired,
        TimingWindow::Live,
    )
    .await;
    assert_unavailable(&resolver, &expired_instance).await;

    let near_expiry = seed_service_inner(
        &pool,
        false,
        TimingWindow::Near,
        TimingWindow::Live,
        TimingWindow::Live,
    )
    .await;
    assert_unavailable(&resolver, &near_expiry).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn execution_target_rejects_revoked_exact_inbound_secret_lease() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_service_with_secret_lease(&pool).await;
    let resolver = PostgresGatewayExecutionTargetResolver::new(worker_pool().await);
    resolver
        .resolve_execution_target(
            fixture.invocation,
            fixture.route,
            fixture.revision,
            &fixture.owner,
        )
        .await
        .expect("active inbound lease permits target");
    sqlx::query(
        "UPDATE gateway_secret_leases
            SET status = 'revoked', revoked_at = now()
          WHERE id = $1",
    )
    .bind(fixture.secret_lease)
    .execute(&pool)
    .await
    .expect("revoke inbound lease");
    assert_unavailable(&resolver, &fixture).await;

    let expired = seed_service_inner(
        &pool,
        true,
        TimingWindow::Live,
        TimingWindow::Live,
        TimingWindow::Expired,
    )
    .await;
    assert_unavailable(&resolver, &expired).await;
}
