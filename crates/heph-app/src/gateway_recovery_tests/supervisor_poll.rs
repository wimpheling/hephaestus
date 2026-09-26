use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
// The supervisor is deliberately moved into the parent-owned loop future;
// its shutdown path runs there rather than at this test scope's end.
#[allow(clippy::significant_drop_tightening)]
async fn daemon_loop_polls_service_supervisor_while_caddy_reconcile_is_blocked() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    sqlx::query(
        "UPDATE gateways
            SET desired_service_revision_id = $2,
                active_revision_id = NULL
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .execute(&pool)
    .await
    .expect("set desired service revision for supervisor startup");
    sqlx::query("DELETE FROM gateway_service_instances WHERE id = $1")
        .bind(fixture.service_instance)
        .execute(&pool)
        .await
        .expect("remove fixture instance before supervisor claim");

    let recovery_pool = database.worker.clone();
    let host_id = format!("recovery-test-{}", fixture.gateway.simple());
    let destroyed = Arc::new(AtomicUsize::new(0));
    let ownership = Arc::new(PostgresGatewayServiceOwnership::new(recovery_pool.clone()));
    let failure_store = Arc::new(PostgresGatewayServiceFailureStore::new(
        recovery_pool.clone(),
    ));
    let resolver = Arc::new(NoopLaunchResolver);
    let targets = Arc::new(PostgresGatewayServiceTargets::new(recovery_pool.clone()));
    let provider = Arc::new(ServiceTransportProvider {
        inner: FakeProvider::new(),
        provisioned: Arc::new(AtomicUsize::new(0)),
        destroyed: Arc::clone(&destroyed),
        destroy_gate: None,
        event_sender: Arc::new(Mutex::new(None)),
        inject_events: false,
    });
    let policy = GatewayServiceSupervisorPolicy::default();
    let owner = GatewayServiceOwner::new(host_id, Uuid::new_v4()).expect("test supervisor owner");
    let supervisor_context = GatewayServiceSupervisorContext {
        owner: owner.clone(),
        policy,
        ownership: ownership.clone(),
        failure_store: failure_store.clone(),
        resolver: resolver.clone(),
        provider: provider.clone(),
        targets: targets.clone(),
        registry: GatewayServiceRegistry::new(10, 16).expect("test service registry"),
        service_authority: String::from("127.0.0.1:8080"),
    };
    let supervisor = GatewayServiceSupervisor::new(supervisor_context)
        .expect("construct test service supervisor");
    let boot = GatewayServiceBootRecovery::new(GatewayServiceBootRecoveryContext {
        owner,
        cleanup_policy: GatewayServiceCleanupDriverPolicy {
            lease: policy.lease,
            database_timeout: policy.instance.probe_timeout,
        },
        shutdown_timeout: policy.instance.shutdown_timeout,
        ownership: ownership.clone(),
        exact_recovery: ownership.clone(),
        targets: targets.clone(),
        failure_store,
        resolver,
        provider: provider.clone(),
    })
    .expect("construct test boot recovery");
    let caddy_started = Arc::new(tokio::sync::Notify::new());
    let caddy_release = Arc::new(tokio::sync::Notify::new());
    let caddy = Arc::new(BlockingCaddyProvider {
        started: Arc::clone(&caddy_started),
        release: Arc::clone(&caddy_release),
        reconciles: Arc::new(AtomicUsize::new(0)),
    });
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(gateway_reconciliation_loop_with_boot(
        make_authority(pool.clone()),
        make_authority(recovery_pool),
        supervisor,
        Some(boot),
        None,
        None,
        targets,
        caddy,
        cancellation.clone(),
    ));

    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts and remains blocked");
    let ready = tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let row: Option<(String, Option<Uuid>)> = sqlx::query_as(
                "SELECT i.state, g.active_revision_id
                   FROM gateway_service_instances AS i
                   JOIN gateways AS g ON g.id = i.gateway_id
                  WHERE i.gateway_id = $1 AND i.revision_id = $2
                  ORDER BY i.fencing_token DESC
                  LIMIT 1",
            )
            .bind(fixture.gateway)
            .bind(fixture.revision)
            .fetch_optional(&pool)
            .await
            .expect("read loop-selected service state");
            if row.as_ref().is_some_and(|(state, active_revision)| {
                state == "ready" && *active_revision == Some(fixture.revision)
            }) {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await;
    let debug_instances: Vec<DebugServiceInstanceRow> = sqlx::query_as(
        "SELECT id, state, fencing_token, failure_code, exit_code
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_all(&pool)
    .await
    .expect("read supervisor debug state");
    assert!(
        ready.is_ok(),
        "supervisor reaches Ready while Caddy is blocked; instances={debug_instances:?}",
    );
    let active_revision: Option<Uuid> =
        sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
            .bind(fixture.gateway)
            .fetch_one(&pool)
            .await
            .expect("read promoted active revision");
    assert_eq!(active_revision, Some(fixture.revision));

    let first_heartbeat: OffsetDateTime = sqlx::query_scalar(
        "SELECT heartbeat_at
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("read ready service heartbeat");
    tokio::time::sleep(StdDuration::from_secs(6)).await;
    let second_heartbeat: OffsetDateTime = sqlx::query_scalar(
        "SELECT heartbeat_at
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("read renewed service heartbeat");
    assert!(
        second_heartbeat > first_heartbeat,
        "supervisor lease monitor must renew while Caddy is blocked"
    );

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("supervisor and Caddy tasks join on shutdown")
        .expect("reconciliation task join");
    caddy_release.notify_one();
    let final_state: String = sqlx::query_scalar(
        "SELECT state
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2
          ORDER BY fencing_token DESC
          LIMIT 1",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("read cleaned service state");
    assert_eq!(final_state, "cleaned");
    assert_eq!(destroyed.load(Ordering::Acquire), 1);
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_restores_active_service_without_manual_start() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let (task, cancellation, caddy_started, caddy_release, destroyed, _provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts and remains blocked");
    let ready_and_active = wait_for_ready_and_active(&pool, fixture).await;
    assert!(
        ready_and_active,
        "active service is restored and promoted by the target scan: gateway={} revision={}",
        fixture.gateway, fixture.revision,
    );
    let active_revision: Option<Uuid> =
        sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
            .bind(fixture.gateway)
            .fetch_one(&pool)
            .await
            .expect("read restored active revision");
    assert_eq!(active_revision, Some(fixture.revision));
    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("active restore loop joins")
        .expect("active restore loop task");
    caddy_release.notify_one();
    assert_eq!(destroyed.load(Ordering::Acquire), 1);
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}
