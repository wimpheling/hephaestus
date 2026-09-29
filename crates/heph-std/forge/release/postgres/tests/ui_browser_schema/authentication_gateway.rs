use super::*;

// These transitions intentionally stay ordered because each assertion restores
// the gateway state used by the next route check.
#[allow(clippy::needless_borrow, clippy::too_many_lines)]
pub async fn assert_gateway_lifecycle(ctx: &AuthenticationContext) {
    let worker = ctx.worker.clone();
    let fixture = &ctx.fixture;
    let store = &ctx.store;
    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(fixture.managed_gateway)
        .execute(&worker)
        .await
        .expect("pause managed/API gateway");
    for (generation, secret, request_route) in [
        (
            fixture.generation,
            test_secret(fixture.actor, 90),
            UiBrowserRequestRoute::Api {
                route: RoutePath::parse("/service/api").expect("static API route"),
                method: HttpMethod::Post,
            },
        ),
        (
            fixture.managed_generation,
            test_secret(fixture.actor, 94),
            UiBrowserRequestRoute::Managed {
                route: UiBrowserRoute::parse("schema-managed").expect("managed route"),
            },
        ),
    ] {
        assert_eq!(
            authenticate_request(&store, generation, secret, request_route).await,
            Err(UiBrowserSessionError::Unauthenticated)
        );
    }
    sqlx::query("UPDATE gateways SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.managed_gateway)
        .execute(&worker)
        .await
        .expect("restore managed/API gateway");

    let cutover_revision = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_revisions
     (id, gateway_id, project_id, repository_id, release_id,
      release_agent_id, release_agent_key, handler_contract, exposure,
      parameters, secret_slots, mailbox_slots, service_loopback_port,
      service_readiness_path, service_health_path, service_log_capture_mode,
      normalized_hash, created_by)
     SELECT $1, gateway_id, project_id, repository_id, release_id,
            release_agent_id, release_agent_key, handler_contract, exposure,
            parameters, secret_slots, mailbox_slots, service_loopback_port,
            service_readiness_path, service_health_path, service_log_capture_mode,
            $2, created_by
     FROM gateway_revisions WHERE id = $3",
    )
    .bind(cutover_revision)
    .bind(vec![6_u8; 32])
    .bind(fixture.managed_revision)
    .execute(&worker)
    .await
    .expect("seed gateway revision cutover");
    sqlx::query(
        "INSERT INTO gateway_routes
     (id, gateway_revision_id, gateway_id, project_id, path, methods)
     SELECT $1, $2, gateway_id, project_id, path, methods
     FROM gateway_routes WHERE gateway_revision_id = $3",
    )
    .bind(Uuid::new_v4())
    .bind(cutover_revision)
    .bind(fixture.managed_revision)
    .execute(&worker)
    .await
    .expect("seed cutover gateway route");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(fixture.managed_gateway)
        .bind(cutover_revision)
        .execute(&worker)
        .await
        .expect("activate gateway revision cutover");
    for (generation, secret, request_route) in [
        (
            fixture.generation,
            test_secret(fixture.actor, 90),
            UiBrowserRequestRoute::Api {
                route: RoutePath::parse("/service/api").expect("static API route"),
                method: HttpMethod::Post,
            },
        ),
        (
            fixture.managed_generation,
            test_secret(fixture.actor, 94),
            UiBrowserRequestRoute::Managed {
                route: UiBrowserRoute::parse("schema-managed").expect("managed route"),
            },
        ),
    ] {
        assert_eq!(
            authenticate_request(&store, generation, secret, request_route).await,
            Err(UiBrowserSessionError::Unauthenticated)
        );
    }
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(fixture.managed_gateway)
        .bind(fixture.managed_revision)
        .execute(&worker)
        .await
        .expect("restore gateway revision");
}
