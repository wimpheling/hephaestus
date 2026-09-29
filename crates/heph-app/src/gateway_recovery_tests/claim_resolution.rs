use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_resolves_lost_claim_before_replacement_admission() {
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
    let observing_ownership = Arc::new(ObservingOwnership::new(database.worker.clone()));
    observing_ownership.lose_claim_ack_for_revision(desired_revision);
    let (task, cancellation, caddy_started, caddy_release, destroyed, provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            false,
            false,
            Some(desired_revision),
            None,
            Some(observing_ownership.clone() as Arc<dyn GatewayServiceOwnership>),
            None,
            None,
            None,
            Some(observing_ownership.clone() as Arc<dyn GatewayServiceClaimResolutionStore>),
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts while claim resolution is pending");

    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let state: Option<String> = sqlx::query_scalar(
                "SELECT state
                   FROM gateway_service_instances
                  WHERE gateway_id = $1 AND revision_id = $2
                  ORDER BY fencing_token DESC LIMIT 1",
            )
            .bind(fixture.gateway)
            .bind(desired_revision)
            .fetch_optional(&pool)
            .await
            .expect("read resolved lost-claim state");
            if state.as_deref() == Some("cleaned") {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("committed lost claim is resolved and cleaned");

    sqlx::query(
        "UPDATE gateways
            SET desired_service_revision_id = $2
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(replacement_revision)
    .execute(&pool)
    .await
    .expect("select replacement after claim cleanup");
    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let active: Option<Uuid> =
                sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
                    .bind(fixture.gateway)
                    .fetch_one(&pool)
                    .await
                    .expect("read replacement active revision");
            if active == Some(replacement_revision) && wait_for_ready(&pool, replacement).await {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("replacement is admitted after the resolved claim releases capacity");
    assert!(
        provisioned.load(Ordering::Acquire) >= 1,
        "replacement admission provisions a VM after lost-claim recovery"
    );

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("lost-claim reconciliation joins")
        .expect("lost-claim reconciliation task");
    caddy_release.notify_one();
    assert!(destroyed.load(Ordering::Acquire) >= 1);
    cleanup_startup_fixture(&pool, desired).await;
    cleanup_startup_fixture(&pool, replacement).await;
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_confirms_absent_claim_before_next_candidate() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let absent_revision = seed_service_candidate(&pool, fixture).await;
    let replacement_revision = seed_service_candidate(&pool, fixture).await;
    let absent = Fixture {
        revision: absent_revision,
        ..fixture
    };
    let replacement = Fixture {
        revision: replacement_revision,
        ..fixture
    };
    let observing_ownership = Arc::new(ObservingOwnership::new(database.worker.clone()));
    observing_ownership.confirm_next_claim_absent();
    let absence_returned = observing_ownership.claim_resolution_returned();
    let (task, cancellation, caddy_started, caddy_release, destroyed, provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            false,
            false,
            Some(absent_revision),
            None,
            Some(observing_ownership.clone() as Arc<dyn GatewayServiceOwnership>),
            None,
            None,
            None,
            Some(observing_ownership.clone() as Arc<dyn GatewayServiceClaimResolutionStore>),
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts while absent claim is resolved");
    tokio::time::timeout(StdDuration::from_secs(10), absence_returned.notified())
        .await
        .expect("serialized claim resolution returns confirmed absence");
    sqlx::query(
        "UPDATE gateways
            SET desired_service_revision_id = $2
          WHERE id = $1",
    )
    .bind(fixture.gateway)
    .bind(replacement_revision)
    .execute(&pool)
    .await
    .expect("select candidate after confirmed absence");
    let absent_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(absent_revision)
    .fetch_one(&pool)
    .await
    .expect("read confirmed absent claim");
    assert_eq!(absent_count, 0, "rolled-back claim leaves no durable row");
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
    .expect("next candidate starts after absent claim releases capacity");
    assert_eq!(provisioned.load(Ordering::Acquire), 1);

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("absent-claim reconciliation joins")
        .expect("absent-claim reconciliation task");
    caddy_release.notify_one();
    assert!(destroyed.load(Ordering::Acquire) >= 1);
    cleanup_startup_fixture(&pool, absent).await;
    cleanup_startup_fixture(&pool, replacement).await;
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}
