use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_failed_desired_service_preserves_active_revision() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let desired_revision = seed_service_candidate(&pool, fixture).await;
    let desired_fixture = Fixture {
        revision: desired_revision,
        service_instance: None,
        ..fixture
    };
    let (task, cancellation, caddy_started, caddy_release, destroyed, provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            true,
            false,
            Some(desired_revision),
            Some(desired_revision),
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
    assert!(
        wait_for_ready(&pool, fixture).await,
        "active service is ready"
    );

    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let failed_attempts: i64 = sqlx::query_scalar(
                "SELECT count(*)
                   FROM gateway_service_instances
                  WHERE gateway_id = $1 AND revision_id = $2 AND failure_code IS NOT NULL",
            )
            .bind(fixture.gateway)
            .bind(desired_revision)
            .fetch_one(&pool)
            .await
            .expect("read failed desired service attempt");
            if failed_attempts > 0 {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("failed desired launch is durably recorded");

    let active_revision: Option<Uuid> =
        sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
            .bind(fixture.gateway)
            .fetch_one(&pool)
            .await
            .expect("read active revision after failed desired launch");
    assert_eq!(active_revision, Some(fixture.revision));
    assert!(wait_for_ready(&pool, fixture).await);
    let desired_ready: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'",
    )
    .bind(fixture.gateway)
    .bind(desired_revision)
    .fetch_one(&pool)
    .await
    .expect("read failed desired readiness");
    assert_eq!(desired_ready, 0);
    assert_eq!(provisioned.load(Ordering::Acquire), 1);

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("failed desired loop joins")
        .expect("failed desired loop task");
    caddy_release.notify_one();
    assert_eq!(destroyed.load(Ordering::Acquire), 1);
    cleanup_startup_fixture(&pool, desired_fixture).await;
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_promotes_desired_service_then_drains_previous_revision() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let desired_revision = seed_service_candidate(&pool, fixture).await;
    let third_revision = seed_service_candidate(&pool, fixture).await;
    let observing_targets = Arc::new(ObservingTargets::new(
        database.worker.clone(),
        third_revision,
    ));
    let destroy_gate = Arc::new(DestroyGate::new());
    let (task, cancellation, caddy_started, caddy_release, destroyed, provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            true,
            false,
            Some(desired_revision),
            None,
            None,
            Some(observing_targets.clone() as Arc<dyn GatewayServiceTargetStore>),
            Some(destroy_gate.clone()),
            None,
            None,
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("Caddy reconciliation starts and remains blocked");
    assert!(
        wait_for_ready(&pool, fixture).await,
        "active service is restored before binding the accepted invocation"
    );

    let active_instance: (Uuid, i64) = sqlx::query_as(
        "SELECT id, fencing_token
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("read restored active service instance");
    let bound_fixture = Fixture {
        service_instance: Some(active_instance.0),
        ..fixture
    };
    let invocation = insert_invocation(&pool, bound_fixture, OffsetDateTime::now_utc()).await;

    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let active: Option<Uuid> =
                sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
                    .bind(fixture.gateway)
                    .fetch_one(&pool)
                    .await
                    .expect("read promoted desired revision");
            if active == Some(desired_revision)
                && wait_for_ready(
                    &pool,
                    Fixture {
                        revision: desired_revision,
                        ..fixture
                    },
                )
                .await
            {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("desired revision becomes active and ready");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let old_state: String = sqlx::query_scalar(
                "SELECT state
                   FROM gateway_service_instances
                  WHERE id = $1",
            )
            .bind(active_instance.0)
            .fetch_one(&pool)
            .await
            .expect("read draining previous revision");
            if old_state == "draining" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("previous revision starts graceful drain");
    assert!(provisioned.load(Ordering::Acquire) >= 2);

    let retained_state: String =
        sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
            .bind(active_instance.0)
            .fetch_one(&pool)
            .await
            .expect("read retained old service");
    assert_eq!(retained_state, "draining");

    let scans_before_third = observing_targets.observed_scans.load(Ordering::Acquire);
    sqlx::query("UPDATE gateways SET desired_service_revision_id = $2 WHERE id = $1")
        .bind(fixture.gateway)
        .bind(third_revision)
        .execute(&pool)
        .await
        .expect("declare third replacement while old service drains");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        while observing_targets.observed_scans.load(Ordering::Acquire) < scans_before_third + 2 {
            observing_targets.observed.notified().await;
        }
    })
    .await
    .expect("two target refreshes observe the third desired revision");
    assert_eq!(
        provisioned.load(Ordering::Acquire),
        2,
        "a third service must wait while the old revision retains capacity"
    );
    let third_instances: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(third_revision)
    .fetch_one(&pool)
    .await
    .expect("read third replacement instances");
    assert_eq!(third_instances, 0);
    sqlx::query("SELECT gateway_invocation_complete($1, 'completed')")
        .bind(invocation)
        .fetch_one(&pool)
        .await
        .expect("complete accepted invocation");
    sqlx::query("DELETE FROM gateway_invocations WHERE id = $1")
        .bind(invocation)
        .execute(&pool)
        .await
        .expect("remove completed test invocation");
    tokio::time::timeout(StdDuration::from_secs(10), destroy_gate.entered.notified())
        .await
        .expect("old service reaches the physical destroy barrier");
    let scans_at_destroy = observing_targets.observed_scans.load(Ordering::Acquire);
    tokio::time::timeout(StdDuration::from_secs(10), async {
        while observing_targets.observed_scans.load(Ordering::Acquire) < scans_at_destroy + 2 {
            observing_targets.observed.notified().await;
        }
    })
    .await
    .expect("two target refreshes complete while old physical cleanup is blocked");
    let old_state_while_blocked: String =
        sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
            .bind(active_instance.0)
            .fetch_one(&pool)
            .await
            .expect("read old state while physical cleanup is blocked");
    assert_eq!(old_state_while_blocked, "stopping");
    let retained_instances: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(third_revision)
    .fetch_one(&pool)
    .await
    .expect("read third instances while old destroy is blocked");
    assert_eq!(
        retained_instances, 0,
        "the next revision remains unclaimed while old physical cleanup is blocked"
    );
    assert_eq!(
        provisioned.load(Ordering::Acquire),
        2,
        "the next revision is not provisioned while old physical cleanup is blocked"
    );
    destroy_gate.release.notify_one();
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
                    .bind(active_instance.0)
                    .fetch_one(&pool)
                    .await
                    .expect("read retired old service");
            if state == "cleaned" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("old service cleans after accepted invocation completes");
    tokio::time::timeout(StdDuration::from_secs(15), async {
        loop {
            let active: Option<Uuid> =
                sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
                    .bind(fixture.gateway)
                    .fetch_one(&pool)
                    .await
                    .expect("read third promoted revision");
            if active == Some(third_revision)
                && wait_for_ready(
                    &pool,
                    Fixture {
                        revision: third_revision,
                        ..fixture
                    },
                )
                .await
            {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("third revision is admitted after old cleanup");

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("cutover loop joins")
        .expect("cutover loop task");
    caddy_release.notify_one();
    assert!(destroyed.load(Ordering::Acquire) >= 2);
    cleanup_startup_fixture(&pool, fixture).await;
    drop(observing_targets);
    pool.close().await;
    drop_isolated_startup_database(database).await;
}
