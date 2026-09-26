use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn coordinator_promotes_ready_service_and_cleans_real_ownership() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let candidate = seed_service_revision(&pool, fixture.gateway).await;
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = NULL, desired_service_revision_id = $2
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(candidate)
    .execute(&pool)
    .await
    .expect("desired candidate");

    let worker = coordinator_worker_pool().await;
    let ownership = Arc::new(PostgresGatewayServiceOwnership::new(worker.clone()));
    let failures = Arc::new(PostgresGatewayServiceFailureStore::new(worker.clone()));
    let targets = Arc::new(PostgresGatewayServiceTargets::new(worker));
    let owner = GatewayServiceOwner::new("coordinator-db-host", Uuid::new_v4()).expect("owner");
    let claim_started = Instant::now();
    let lease = ownership
        .claim_new(fixture.gateway, candidate, &owner, Duration::from_secs(30))
        .await
        .expect("claim desired candidate");
    let provider = Arc::new(FakeProvider::new(lease.identity));
    let resolver = Arc::new(FakeResolver::new(lease.identity, Arc::clone(&provider.vm)));
    let registry = GatewayServiceRegistry::new(2, 2).expect("registry");
    let (coordinator, control) = GatewayServiceCoordinator::new(
        lease.clone(),
        owner.clone(),
        claim_started + Duration::from_secs(30),
        claim_started + Duration::from_secs(10),
        GatewayServiceStartupIntent::ActivateDesired,
        ownership,
        failures,
        resolver.clone(),
        provider.clone(),
        targets,
        registry.clone(),
        "service.test",
        coordinator_policy(
            ServiceInstancePolicy::new(
                Duration::from_secs(5),
                Duration::from_millis(5),
                Duration::from_secs(1),
                Duration::from_secs(1),
            ),
            GatewayServiceLeasePolicy {
                lease_duration: Duration::from_secs(30),
                renewal_interval: Duration::from_secs(5),
            },
        ),
    )
    .expect("coordinator");
    let mut status = control.subscribe();
    let task = tokio::spawn(coordinator.run());
    wait_for_status(&mut status, GatewayServiceCoordinatorStatus::Probing).await;
    provider.vm.release_readiness();
    wait_for_status(&mut status, GatewayServiceCoordinatorStatus::Ready).await;

    assert_eq!(
        active_pointer(&pool, fixture.gateway).await,
        Some(candidate)
    );
    let state: String =
        sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
            .bind(lease.identity.instance_id)
            .fetch_one(&pool)
            .await
            .expect("ready state");
    assert_eq!(state, "ready");
    let key = GatewayServiceInstanceKey {
        identity: lease.identity,
        fencing_token: lease.fencing_token,
    };
    let response = registry
        .exchange(
            key,
            GatewayRequest {
                method: Method::GET,
                path_and_query: String::from("/ready"),
                headers: HeaderMap::new(),
                body: Vec::new().into(),
                trusted: TrustedRequestMetadata {
                    scheme: GatewayScheme::Http,
                    authority: String::from("service.test"),
                    client_address: "127.0.0.1".parse().expect("client address"),
                    request_id: Uuid::new_v4(),
                },
            },
            ServiceHttpPolicy::from_gateway_limits(GatewayLimits {
                max_request_body_bytes: 1024,
                max_response_body_bytes: 1024,
                max_request_headers: 16,
                max_response_headers: 16,
                max_path_and_query_bytes: 256,
                execution_timeout: Duration::from_secs(1),
            }),
        )
        .await
        .expect("registry exchange");
    assert_eq!(response.status, StatusCode::OK);

    drop(registry);
    control.cancel();
    assert!(
        timeout(Duration::from_secs(5), task)
            .await
            .expect("coordinator shutdown")
            .expect("coordinator join")
            .is_ok()
    );
    let final_state: String =
        sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
            .bind(lease.identity.instance_id)
            .fetch_one(&pool)
            .await
            .expect("cleaned state");
    assert_eq!(final_state, "cleaned");
    let remaining_sessions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_runtime_authority_sessions
          WHERE gateway_id = $1 AND gateway_revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(candidate)
    .fetch_one(&pool)
    .await
    .expect("session-free service startup");
    assert_eq!(remaining_sessions, 0);
    assert_eq!(provider.vm.destroys.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    assert!(resolver.cleanup_after_destroy.load(Ordering::Relaxed));
}

#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn coordinator_restores_old_active_without_overwriting_new_desired_revision() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let old_active = seed_service_revision(&pool, fixture.gateway).await;
    let desired = seed_service_revision(&pool, fixture.gateway).await;
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = $2, desired_service_revision_id = $3
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(old_active)
    .bind(desired)
    .execute(&pool)
    .await
    .expect("active and desired revisions");

    let worker = coordinator_worker_pool().await;
    let ownership = Arc::new(PostgresGatewayServiceOwnership::new(worker.clone()));
    let failures = Arc::new(PostgresGatewayServiceFailureStore::new(worker.clone()));
    let targets = Arc::new(PostgresGatewayServiceTargets::new(worker));
    let owner =
        GatewayServiceOwner::new("coordinator-restore-host", Uuid::new_v4()).expect("owner");
    let claim_started = Instant::now();
    let lease = ownership
        .claim_new(fixture.gateway, old_active, &owner, Duration::from_secs(30))
        .await
        .expect("claim old active revision");
    let provider = Arc::new(FakeProvider::new(lease.identity));
    let resolver = Arc::new(FakeResolver::new(lease.identity, Arc::clone(&provider.vm)));
    let (coordinator, control) = GatewayServiceCoordinator::new(
        lease.clone(),
        owner,
        claim_started + Duration::from_secs(30),
        claim_started + Duration::from_secs(10),
        GatewayServiceStartupIntent::RestoreActive,
        ownership,
        failures,
        resolver.clone(),
        provider.clone(),
        targets,
        GatewayServiceRegistry::new(2, 2).expect("registry"),
        "service.test",
        coordinator_policy(
            ServiceInstancePolicy::new(
                Duration::from_secs(5),
                Duration::from_millis(5),
                Duration::from_secs(1),
                Duration::from_secs(1),
            ),
            GatewayServiceLeasePolicy {
                lease_duration: Duration::from_secs(30),
                renewal_interval: Duration::from_secs(5),
            },
        ),
    )
    .expect("coordinator");
    let mut status = control.subscribe();
    let task = tokio::spawn(coordinator.run());
    wait_for_status(&mut status, GatewayServiceCoordinatorStatus::Probing).await;
    provider.vm.release_readiness();
    wait_for_status(&mut status, GatewayServiceCoordinatorStatus::Ready).await;
    assert_eq!(
        active_pointer(&pool, fixture.gateway).await,
        Some(old_active)
    );
    let desired_pointer: Option<Uuid> =
        sqlx::query_scalar("SELECT desired_service_revision_id FROM gateways WHERE id = $1")
            .bind(fixture.gateway)
            .fetch_one(&pool)
            .await
            .expect("desired pointer");
    assert_eq!(desired_pointer, Some(desired));

    control.cancel();
    assert!(
        timeout(Duration::from_secs(5), task)
            .await
            .expect("coordinator shutdown")
            .expect("coordinator join")
            .is_ok()
    );
    let final_state: String =
        sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
            .bind(lease.identity.instance_id)
            .fetch_one(&pool)
            .await
            .expect("cleaned restore state");
    assert_eq!(final_state, "cleaned");
    let remaining_sessions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_runtime_authority_sessions
          WHERE gateway_id = $1 AND gateway_revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(old_active)
    .fetch_one(&pool)
    .await
    .expect("session-free restored startup");
    assert_eq!(remaining_sessions, 0);
    assert!(resolver.cleanup_after_destroy.load(Ordering::Relaxed));
}
