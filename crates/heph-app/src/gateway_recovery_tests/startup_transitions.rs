use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_starts_valid_desired_service_after_revoked_active_cleans() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let desired_revision = seed_service_candidate(&pool, fixture).await;
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
    let active_release: Uuid =
        sqlx::query_scalar("SELECT release_id FROM gateway_revisions WHERE id = $1")
            .bind(fixture.revision)
            .fetch_one(&pool)
            .await
            .expect("read active release");
    sqlx::query("UPDATE releases SET state = 'revoked', revoked_at = now() WHERE id = $1")
        .bind(active_release)
        .execute(&pool)
        .await
        .expect("revoke active release");

    tokio::time::timeout(StdDuration::from_secs(20), async {
        loop {
            let active: Option<Uuid> =
                sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
                    .bind(fixture.gateway)
                    .fetch_one(&pool)
                    .await
                    .expect("read replacement active revision");
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
    .expect("valid desired service starts after revoked active cleanup");
    assert!(provisioned.load(Ordering::Acquire) >= 2);

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("revoked-active loop joins")
        .expect("revoked-active loop task");
    caddy_release.notify_one();
    assert!(destroyed.load(Ordering::Acquire) >= 2);
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_loop_retries_drain_after_observed_stale_target_conflict() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let observing_ownership = Arc::new(ObservingOwnership::new(database.worker.clone()));
    let (task, cancellation, caddy_started, caddy_release, destroyed, _provisioned) =
        spawn_automatic_start_with_worker(
            &pool,
            &database.worker,
            fixture,
            true,
            false,
            None,
            None,
            Some(observing_ownership.clone() as Arc<dyn GatewayServiceOwnership>),
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
    let active_instance: Uuid = sqlx::query_scalar(
        "SELECT id FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'
          ORDER BY fencing_token DESC LIMIT 1",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("read active instance for drain hold");
    let invocation = insert_invocation(
        &pool,
        Fixture {
            service_instance: Some(active_instance),
            ..fixture
        },
        OffsetDateTime::now_utc(),
    )
    .await;

    let draining_entered = observing_ownership.draining_entered.notified();
    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(fixture.gateway)
        .execute(&pool)
        .await
        .expect("pause gateway before stale target read");
    let mut transition = pool.begin().await.expect("begin lifecycle transition");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *transition)
        .await
        .expect("read lifecycle lock holder pid");
    sqlx::query("SELECT id FROM gateways WHERE id = $1 FOR UPDATE")
        .bind(fixture.gateway)
        .fetch_one(&mut *transition)
        .await
        .expect("hold gateway transition lock");
    sqlx::query("UPDATE gateways SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.gateway)
        .execute(&mut *transition)
        .await
        .expect("restore gateway while transition lock is held");

    tokio::time::timeout(StdDuration::from_secs(10), draining_entered)
        .await
        .expect("the exact ownership adapter observed mark_draining entry");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let waiting: Option<i32> = sqlx::query_scalar(
                "SELECT pid
                   FROM pg_stat_activity
                  WHERE application_name = 'gateway-recovery-test'
                    AND wait_event_type = 'Lock'
                    AND $1 = ANY(pg_blocking_pids(pid))
                  LIMIT 1",
            )
            .bind(holder_pid)
            .fetch_optional(&pool)
            .await
            .expect("inspect stale drain lock wait");
            if waiting.is_some() {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("mark_draining was blocked by the held gateway row lock");
    let draining_returned = observing_ownership.draining_returned.notified();
    transition
        .commit()
        .await
        .expect("commit concurrent lifecycle transition");

    tokio::time::timeout(StdDuration::from_secs(10), draining_returned)
        .await
        .expect("the exact mark_draining call returned after the lock release");
    assert_eq!(
        *observing_ownership
            .draining_error
            .lock()
            .expect("drain observation lock"),
        Some(GatewayServiceOwnershipError::Conflict),
        "the stale target transition must return Conflict before retrying"
    );
    let state: String = sqlx::query_scalar(
        "SELECT state FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2
          ORDER BY fencing_token DESC LIMIT 1",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("read post-conflict service state");
    assert_eq!(state, "ready", "Conflict leaves the service Ready");

    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(fixture.gateway)
        .execute(&pool)
        .await
        .expect("pause gateway for retry drain");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let state: String = sqlx::query_scalar(
                "SELECT state FROM gateway_service_instances
                  WHERE gateway_id = $1 AND revision_id = $2
                  ORDER BY fencing_token DESC LIMIT 1",
            )
            .bind(fixture.gateway)
            .bind(fixture.revision)
            .fetch_one(&pool)
            .await
            .expect("read retried drain state");
            if state == "draining" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("later exact target refresh retries the drain");

    sqlx::query("SELECT gateway_invocation_complete($1, 'completed')")
        .bind(invocation)
        .fetch_one(&pool)
        .await
        .expect("complete held invocation");
    sqlx::query("DELETE FROM gateway_invocations WHERE id = $1")
        .bind(invocation)
        .execute(&pool)
        .await
        .expect("remove held invocation");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
                    .bind(active_instance)
                    .fetch_one(&pool)
                    .await
                    .expect("read cleaned retried drain state");
            if state == "cleaned" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("retried drain cleans after invocation completion");

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), task)
        .await
        .expect("stale-drain loop joins")
        .expect("stale-drain loop task");
    caddy_release.notify_one();
    assert_eq!(destroyed.load(Ordering::Acquire), 1);
    cleanup_startup_fixture(&pool, fixture).await;
    drop_isolated_startup_database(database).await;
}
