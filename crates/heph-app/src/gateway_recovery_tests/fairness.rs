use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_fairly_refreshes_other_jobs_while_one_target_lookup_times_out() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let first = seed_fixture(&pool, "http.service.v1").await;
    let second = seed_fixture(&pool, "http.service.v1").await;
    sqlx::query("DELETE FROM gateway_service_instances WHERE id = $1")
        .bind(second.service_instance)
        .execute(&pool)
        .await
        .expect("remove second fixture instance before automatic startup");
    sqlx::query(
        "UPDATE gateways
            SET active_revision_id = $2, desired_service_revision_id = $2
          WHERE id = $1",
    )
    .bind(second.gateway)
    .bind(second.revision)
    .execute(&pool)
    .await
    .expect("set second active service target");
    let targets = Arc::new(FairnessTargets::new(
        database.worker.clone(),
        first.gateway,
        first.revision,
    ));
    let (task, cancellation, caddy_started, caddy_release, destroyed, _provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            first,
            true,
            false,
            None,
            None,
            None,
            Some(targets.clone() as Arc<dyn GatewayServiceTargetStore>),
            None,
            None,
            None,
        )
        .await;
    tokio::time::timeout(StdDuration::from_secs(10), caddy_started.notified())
        .await
        .expect("initial Caddy reconciliation starts");
    let caddy_second = caddy_started.notified();
    caddy_release.notify_one();
    tokio::time::timeout(StdDuration::from_secs(10), caddy_second)
        .await
        .expect("Caddy reconciliation makes a later pass");
    assert!(wait_for_ready(&pool, first).await, "first service is ready");
    assert!(
        wait_for_ready(&pool, second).await,
        "second service is ready"
    );

    let second_instance: Uuid = sqlx::query_scalar(
        "SELECT id FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'
          ORDER BY fencing_token DESC LIMIT 1",
    )
    .bind(second.gateway)
    .bind(second.revision)
    .fetch_one(&pool)
    .await
    .expect("read second ready instance");
    let invocation = insert_invocation(
        &pool,
        Fixture {
            service_instance: Some(second_instance),
            ..second
        },
        OffsetDateTime::now_utc(),
    )
    .await;
    let first_instance: Uuid = sqlx::query_scalar(
        "SELECT id FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'
          ORDER BY fencing_token DESC LIMIT 1",
    )
    .bind(first.gateway)
    .bind(first.revision)
    .fetch_one(&pool)
    .await
    .expect("read first ready instance");
    let blocked_wait = targets.blocked_entered.notified();
    targets.blocked.store(true, Ordering::Release);
    tokio::time::timeout(StdDuration::from_secs(10), blocked_wait)
        .await
        .expect("first exact target lookup is blocked");
    let heartbeat_before: OffsetDateTime =
        sqlx::query_scalar("SELECT heartbeat_at FROM gateway_service_instances WHERE id = $1")
            .bind(first_instance)
            .fetch_one(&pool)
            .await
            .expect("read first heartbeat after blocked refresh starts");
    let other_gets_before = targets.other_gets.load(Ordering::Acquire);
    let blocked_dropped = targets.blocked_dropped.notified();
    tokio::time::timeout(StdDuration::from_secs(10), blocked_dropped)
        .await
        .expect("first exact target lookup times out and is dropped");

    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(second.gateway)
        .execute(&pool)
        .await
        .expect("pause second gateway for retirement");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        while targets.other_gets.load(Ordering::Acquire) <= other_gets_before {
            targets.other_observed.notified().await;
        }
    })
    .await
    .expect("refresh cursor advances to the other job");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
                    .bind(second_instance)
                    .fetch_one(&pool)
                    .await
                    .expect("read second draining state");
            if state == "draining" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("second service starts draining after the other refresh");

    let caddy_third = caddy_started.notified();
    caddy_release.notify_one();
    tokio::time::timeout(StdDuration::from_secs(10), caddy_third)
        .await
        .expect("Caddy reconciliation progresses while target lookup is blocked");
    caddy_release.notify_one();

    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let heartbeat: OffsetDateTime = sqlx::query_scalar(
                "SELECT heartbeat_at FROM gateway_service_instances WHERE id = $1",
            )
            .bind(first_instance)
            .fetch_one(&pool)
            .await
            .expect("read renewed first heartbeat");
            if heartbeat > heartbeat_before {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("first service lease renews while exact refresh is blocked");

    sqlx::query("SELECT gateway_invocation_complete($1, 'completed')")
        .bind(invocation)
        .fetch_one(&pool)
        .await
        .expect("complete second accepted invocation");
    sqlx::query("DELETE FROM gateway_invocations WHERE id = $1")
        .bind(invocation)
        .execute(&pool)
        .await
        .expect("remove second invocation");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
                    .bind(second_instance)
                    .fetch_one(&pool)
                    .await
                    .expect("read cleaned second state");
            if state == "cleaned" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("other service cleans while first refresh is blocked");

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("fair refresh loop joins")
        .expect("fair refresh loop task");
    cleanup_startup_fixture(&pool, first).await;
    cleanup_startup_fixture(&pool, second).await;
    drop_isolated_startup_database(database).await;
    assert!(destroyed.load(Ordering::Acquire) >= 2);
}
