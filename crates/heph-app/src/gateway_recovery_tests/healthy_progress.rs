use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_keeps_healthy_service_progressing_while_claim_resolution_is_locked() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let desired_revision = seed_service_candidate(&pool, fixture).await;
    let replacement_revision = seed_service_candidate(&pool, fixture).await;
    let desired = Fixture {
        revision: desired_revision,
        ..fixture
    };
    let replacement = Fixture {
        revision: replacement_revision,
        ..fixture
    };
    let healthy = seed_fixture(&pool, "http.service.v1").await;
    sqlx::query(
        "DELETE FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(healthy.gateway)
    .bind(healthy.revision)
    .execute(&pool)
    .await
    .expect("remove seeded healthy inventory before restore");
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = $2, desired_service_revision_id = NULL
          WHERE id = $1",
    )
    .bind(healthy.gateway)
    .bind(healthy.revision)
    .execute(&pool)
    .await
    .expect("seed unrelated healthy service target");
    let observing_ownership = Arc::new(ObservingOwnership::new(database.worker.clone()));
    observing_ownership.lose_claim_ack_for_revision(desired_revision);
    let claim_fault_consumed = observing_ownership.claim_ack_lost_consumed();
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL")
        .expect("test database URL for claim-resolution observer");
    let claim_options = PgConnectOptions::from_str(&database_url)
        .expect("parse claim-resolution database URL")
        .database(&database.name);
    let claim_pool =
        worker_pool_for_options_named(claim_options, "gateway-claim-resolution-test").await;
    let claim_entered = Arc::new(tokio::sync::Notify::new());
    let (claim_proceed, claim_proceed_rx) = tokio::sync::watch::channel(false);
    let blocking_claim = Arc::new(BlockingClaimResolution {
        inner: Arc::new(PostgresGatewayServiceOwnership::new(claim_pool.clone())),
        entered: Arc::clone(&claim_entered),
        proceed: claim_proceed_rx,
    });
    let observing_targets = Arc::new(ObservingTargets::new(
        database.worker.clone(),
        replacement_revision,
    ));
    let (task, cancellation, caddy_started, caddy_release, destroyed, provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            true,
            false,
            Some(desired_revision),
            None,
            Some(observing_ownership.clone() as Arc<dyn GatewayServiceOwnership>),
            Some(observing_targets.clone() as Arc<dyn GatewayServiceTargetStore>),
            None,
            None,
            Some(blocking_claim as Arc<dyn GatewayServiceClaimResolutionStore>),
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts while claim resolution is pending");
    assert!(
        wait_for_ready(&pool, fixture).await,
        "active service is ready"
    );
    assert!(
        wait_for_ready(&pool, healthy).await,
        "unrelated healthy service is ready"
    );
    let provisioned_before_replacement = provisioned.load(Ordering::Acquire);
    let active_instance: Uuid = sqlx::query_scalar(
        "SELECT id FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'
          ORDER BY fencing_token DESC LIMIT 1",
    )
    .bind(healthy.gateway)
    .bind(healthy.revision)
    .fetch_one(&pool)
    .await
    .expect("read healthy active instance");

    tokio::time::timeout(StdDuration::from_secs(15), claim_fault_consumed.notified())
        .await
        .expect("desired claim acknowledgement is deliberately lost");
    let lost_claim: (Uuid, i64) = sqlx::query_as(
        "SELECT id, fencing_token
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2
          ORDER BY fencing_token DESC LIMIT 1",
    )
    .bind(fixture.gateway)
    .bind(desired_revision)
    .fetch_one(&pool)
    .await
    .expect("capture committed lost claim identity and fence");
    tokio::time::timeout(StdDuration::from_secs(15), claim_entered.notified())
        .await
        .expect("claim resolution attempt starts");
    sqlx::query(
        "UPDATE gateways
            SET desired_service_revision_id = $2
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(replacement_revision)
    .execute(&pool)
    .await
    .expect("select replacement while lost claim remains unresolved");
    let mut gateway_lock = pool.begin().await.expect("begin gateway lock");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gateway_lock)
        .await
        .expect("read gateway lock backend");
    sqlx::query("SELECT id FROM gateways WHERE id = $1 FOR UPDATE")
        .bind(fixture.gateway)
        .fetch_one(&mut *gateway_lock)
        .await
        .expect("hold gateway claim-resolution barrier");
    claim_proceed
        .send(true)
        .expect("open persistent claim-resolution gate");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                    SELECT 1
                      FROM pg_stat_activity AS waiting
                     WHERE waiting.application_name = 'gateway-claim-resolution-test'
                       AND $1 = ANY(pg_blocking_pids(waiting.pid))
                )",
            )
            .bind(holder_pid)
            .fetch_one(&pool)
            .await
            .expect("observe blocked claim resolver");
            if blocked {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("claim resolution is blocked by the gateway row lock");

    let scans_before_replacement = observing_targets.observed_scans.load(Ordering::Acquire);
    let claim_attempts_before_replacement =
        observing_ownership.claim_attempts(replacement_revision);
    assert_eq!(
        claim_attempts_before_replacement, 0,
        "replacement must not be claimed before the blocked-resolution observation"
    );
    tokio::time::timeout(StdDuration::from_secs(10), async {
        while observing_targets.observed_scans.load(Ordering::Acquire)
            < scans_before_replacement.saturating_add(2)
        {
            observing_targets.observed.notified().await;
        }
    })
    .await
    .expect("two target scans complete while claim resolution remains blocked");
    let replacement_instances: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(replacement_revision)
    .fetch_one(&pool)
    .await
    .expect("read blocked replacement instances");
    assert_eq!(replacement_instances, 0);
    assert_eq!(
        observing_ownership.claim_attempts(replacement_revision),
        0,
        "replacement claim_new is not attempted while the lost claim retains capacity"
    );
    assert_eq!(
        provisioned.load(Ordering::Acquire),
        provisioned_before_replacement,
        "replacement does not provision while the lost claim retains capacity"
    );

    let heartbeat_before: OffsetDateTime =
        sqlx::query_scalar("SELECT heartbeat_at FROM gateway_service_instances WHERE id = $1")
            .bind(active_instance)
            .fetch_one(&pool)
            .await
            .expect("read healthy heartbeat after blocker observed");
    let caddy_second = caddy_started.notified();
    caddy_release.notify_one();
    tokio::time::timeout(StdDuration::from_secs(10), caddy_second)
        .await
        .expect("Caddy makes progress while claim resolution is blocked");
    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let heartbeat: OffsetDateTime = sqlx::query_scalar(
                "SELECT heartbeat_at FROM gateway_service_instances WHERE id = $1",
            )
            .bind(active_instance)
            .fetch_one(&pool)
            .await
            .expect("read renewed healthy heartbeat");
            if heartbeat > heartbeat_before {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("healthy lease renews while claim resolution is blocked");
    drop(gateway_lock);

    let claim_cleaned_at: OffsetDateTime =
        tokio::time::timeout(StdDuration::from_secs(15), async {
            loop {
                let row: Option<(String, Option<OffsetDateTime>, i64)> = sqlx::query_as(
                    "SELECT state, cleaned_at, fencing_token
                   FROM gateway_service_instances
                  WHERE id = $1",
                )
                .bind(lost_claim.0)
                .fetch_optional(&pool)
                .await
                .expect("read resolved claim cleanup state");
                if let Some((state, Some(cleaned_at), fencing_token)) = row
                    && state == "cleaned"
                {
                    assert_eq!(fencing_token, lost_claim.1);
                    break cleaned_at;
                }
                tokio::time::sleep(StdDuration::from_millis(50)).await;
            }
        })
        .await
        .expect("blocked claim resolves and cleans after lock release");
    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let (active, ready_count): (Option<Uuid>, i64) = sqlx::query_as(
                "SELECT g.active_revision_id,
                        count(i.id) FILTER (WHERE i.state = 'ready')
                   FROM gateways AS g
                   LEFT JOIN gateway_service_instances AS i
                     ON i.gateway_id = g.id AND i.revision_id = $2
                  WHERE g.id = $1
                  GROUP BY g.active_revision_id",
            )
            .bind(fixture.gateway)
            .bind(replacement_revision)
            .fetch_one(&pool)
            .await
            .expect("read admitted replacement readiness");
            if active == Some(replacement_revision) && ready_count > 0 {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("replacement starts after the retained claim is cleaned");
    let replacement_created_at: OffsetDateTime = sqlx::query_scalar(
        "SELECT min(created_at) FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2
        ",
    )
    .bind(fixture.gateway)
    .bind(replacement_revision)
    .fetch_one(&pool)
    .await
    .expect("read replacement creation time");
    assert!(
        replacement_created_at >= claim_cleaned_at,
        "replacement must be created after the lost claim is durably cleaned"
    );
    assert!(
        observing_ownership.claim_attempts(replacement_revision)
            > claim_attempts_before_replacement,
        "replacement claim_new occurs only after cleanup releases capacity"
    );

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("blocked claim loop joins")
        .expect("blocked claim loop task");
    caddy_release.notify_one();
    assert!(destroyed.load(Ordering::Acquire) >= 1);
    claim_pool.close().await;
    cleanup_startup_fixture(&pool, desired).await;
    cleanup_startup_fixture(&pool, replacement).await;
    cleanup_startup_fixture(&pool, healthy).await;
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}
