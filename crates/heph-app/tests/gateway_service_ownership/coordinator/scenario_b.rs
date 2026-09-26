use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn coordinator_records_readiness_failure_before_cleaning_candidate() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let candidate = seed_service_revision(&pool, fixture.gateway).await;
    sqlx::query(
        "UPDATE gateways
            SET desired_service_revision_id = $2
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
    let owner =
        GatewayServiceOwner::new("coordinator-failure-host", Uuid::new_v4()).expect("owner");
    let claim_started = Instant::now();
    let lease = ownership
        .claim_new(fixture.gateway, candidate, &owner, Duration::from_secs(30))
        .await
        .expect("claim desired candidate");
    let provider = Arc::new(FakeProvider::new(lease.identity));
    let resolver = Arc::new(FakeResolver::new(lease.identity, Arc::clone(&provider.vm)));
    let (coordinator, control) = GatewayServiceCoordinator::new(
        lease.clone(),
        owner,
        claim_started + Duration::from_secs(30),
        claim_started + Duration::from_secs(5),
        GatewayServiceStartupIntent::ActivateDesired,
        ownership,
        failures,
        resolver.clone(),
        provider.clone(),
        targets,
        GatewayServiceRegistry::new(2, 2).expect("registry"),
        "service.test",
        coordinator_policy(
            ServiceInstancePolicy::new(
                Duration::from_secs(1),
                Duration::from_millis(5),
                Duration::from_millis(50),
                Duration::from_secs(1),
            ),
            GatewayServiceLeasePolicy {
                lease_duration: Duration::from_secs(30),
                renewal_interval: Duration::from_secs(5),
            },
        ),
    )
    .expect("coordinator");
    let task = tokio::spawn(coordinator.run());
    let failure = timeout(Duration::from_secs(10), task)
        .await
        .expect("coordinator failure")
        .expect("coordinator join")
        .expect_err("readiness failure");
    control.cancel();
    assert_eq!(
        failure.reason,
        gateway_edge::GatewayServiceCoordinatorFailureReason::Runtime
    );
    assert!(failure.pending_failure.is_none());
    assert!(failure.physical_cleanup_complete);
    assert!(failure.durable_cleanup_complete);
    assert_eq!(provider.vm.destroys.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);

    assert_eq!(
        active_pointer(&pool, fixture.gateway).await,
        Some(fixture.revision)
    );
    let desired_pointer: Option<Uuid> =
        sqlx::query_scalar("SELECT desired_service_revision_id FROM gateways WHERE id = $1")
            .bind(fixture.gateway)
            .fetch_one(&pool)
            .await
            .expect("desired pointer");
    assert_eq!(desired_pointer, Some(candidate));
    let state: (String, Option<String>, i32) = sqlx::query_as(
        "SELECT instance.state, instance.failure_code, retry.failure_streak
           FROM gateway_service_instances AS instance
           JOIN gateway_service_retry_state AS retry
             ON retry.gateway_id = instance.gateway_id
            AND retry.revision_id = instance.revision_id
          WHERE instance.id = $1",
    )
    .bind(lease.identity.instance_id)
    .fetch_one(&pool)
    .await
    .expect("recorded readiness failure");
    assert_eq!(
        state,
        (String::from("cleaned"), Some(String::from("readiness")), 1)
    );
    assert_eq!(GatewayServiceFailureCode::Readiness.as_str(), "readiness");
}
