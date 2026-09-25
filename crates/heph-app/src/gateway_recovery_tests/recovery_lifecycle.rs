use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_reconciliation_reaps_abandoned_service_invocation_and_joins() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let now = OffsetDateTime::now_utc();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let invocation = insert_invocation(&pool, fixture, now).await;
    let session =
        insert_host_session(&pool, fixture, invocation, now, now + Duration::minutes(10)).await;
    let lease = insert_lease(&pool, fixture, invocation, session).await;
    let live = seed_fixture(&pool, "http.service.v1").await;
    let live_invocation = insert_invocation(&pool, live, now).await;
    let live_session = insert_host_session(
        &pool,
        live,
        live_invocation,
        now,
        now + Duration::minutes(10),
    )
    .await;
    let live_lease = insert_lease(&pool, live, live_invocation, live_session).await;
    sqlx::query(
        "UPDATE gateway_service_instances
            SET state = 'stopping'
          WHERE id = $1",
    )
    .bind(fixture.service_instance)
    .execute(&pool)
    .await
    .expect("mark service instance stopping");

    let reconciles = Arc::new(AtomicUsize::new(0));
    let authority = make_authority(pool.clone());
    let recovery_pool = worker_pool().await;
    let recovery_authority = make_authority(recovery_pool.clone());
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(gateway_reconciliation_loop(
        authority,
        recovery_authority,
        test_supervisor_context(recovery_pool),
        Arc::new(RecoveryProvider::new(Arc::clone(&reconciles))),
        cancellation.clone(),
    ));
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let outcome: String =
                sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
                    .bind(invocation)
                    .fetch_one(&pool)
                    .await
                    .expect("read invocation outcome");
            if outcome == "timed_out" {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("daemon recovery reaches the abandoned invocation");
    let session_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_runtime_authority_sessions WHERE id = $1")
            .bind(session)
            .fetch_one(&pool)
            .await
            .expect("read runtime session status");
    assert_eq!(session_status, "revoked");
    let lease_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_secret_leases WHERE id = $1")
            .bind(lease)
            .fetch_one(&pool)
            .await
            .expect("read secret lease status");
    assert_eq!(lease_status, "revoked");
    let live_state: String =
        sqlx::query_scalar("SELECT state FROM gateway_service_instances WHERE id = $1")
            .bind(live.service_instance)
            .fetch_one(&pool)
            .await
            .expect("read live service state");
    assert_eq!(live_state, "ready");
    let live_outcome: String =
        sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
            .bind(live_invocation)
            .fetch_one(&pool)
            .await
            .expect("read live invocation outcome");
    assert_eq!(live_outcome, "accepted");
    let live_session_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_runtime_authority_sessions WHERE id = $1")
            .bind(live_session)
            .fetch_one(&pool)
            .await
            .expect("read live session status");
    assert_eq!(live_session_status, "active");
    let live_lease_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_secret_leases WHERE id = $1")
            .bind(live_lease)
            .fetch_one(&pool)
            .await
            .expect("read live lease status");
    assert_eq!(live_lease_status, "active");

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(5), task)
        .await
        .expect("reconciliation loop joins on shutdown")
        .expect("reconciliation task join");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn daemon_recovery_cancellation_joins_a_database_blocked_batch() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let now = OffsetDateTime::now_utc();
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let invocation = insert_invocation(&pool, fixture, now).await;
    let session =
        insert_host_session(&pool, fixture, invocation, now, now + Duration::minutes(10)).await;
    let _lease = insert_lease(&pool, fixture, invocation, session).await;
    sqlx::query(
        "UPDATE gateway_service_instances
            SET state = 'stopping'
          WHERE id = $1",
    )
    .bind(fixture.service_instance)
    .execute(&pool)
    .await
    .expect("mark service instance stopping");

    let mut lock = pool.begin().await.expect("begin authority lock");
    sqlx::query(
        "SELECT id
           FROM gateway_runtime_authority_sessions
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(session)
    .execute(&mut *lock)
    .await
    .expect("lock runtime authority session");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *lock)
        .await
        .expect("read lock holder backend pid");

    let reconciles = Arc::new(AtomicUsize::new(0));
    let authority = make_authority(pool.clone());
    let recovery_pool = worker_pool().await;
    let recovery_authority = make_authority(recovery_pool.clone());
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(gateway_reconciliation_loop(
        authority,
        recovery_authority,
        test_supervisor_context(recovery_pool),
        Arc::new(RecoveryProvider::new(Arc::clone(&reconciles))),
        cancellation.clone(),
    ));
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let waiters: Vec<(i32, String)> = sqlx::query_as(
                "SELECT pid, application_name
                   FROM pg_stat_activity
                  WHERE wait_event_type = 'Lock'
                    AND $1 = ANY(pg_blocking_pids(pid))",
            )
            .bind(holder_pid)
            .fetch_all(&pool)
            .await
            .expect("inspect PostgreSQL lock wait");
            if waiters
                .iter()
                .any(|(_, application_name)| application_name == "gateway-recovery-test")
            {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("recovery batch waits on the held authority row");
    let reconciles_before = reconciles.load(Ordering::Acquire);
    tokio::time::timeout(StdDuration::from_secs(5), async {
        loop {
            let waiters: Vec<(i32, String)> = sqlx::query_as(
                "SELECT pid, application_name
                   FROM pg_stat_activity
                  WHERE wait_event_type = 'Lock'
                    AND $1 = ANY(pg_blocking_pids(pid))",
            )
            .bind(holder_pid)
            .fetch_all(&pool)
            .await
            .expect("inspect PostgreSQL lock wait");
            let worker_waiting = waiters
                .iter()
                .any(|(_, application_name)| application_name == "gateway-recovery-test");
            if worker_waiting && reconciles.load(Ordering::Acquire) > reconciles_before {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("Caddy reconciliation advances while recovery waits");

    cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(5), task)
        .await
        .expect("blocked recovery loop joins on cancellation")
        .expect("reconciliation task join");
    let outcome: String =
        sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
            .bind(invocation)
            .fetch_one(&pool)
            .await
            .expect("read uncommitted invocation outcome");
    assert_eq!(outcome, "accepted");
    let session_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_runtime_authority_sessions WHERE id = $1")
            .bind(session)
            .fetch_one(&pool)
            .await
            .expect("read uncommitted session status");
    assert_eq!(session_status, "active");
    let lease_status: String =
        sqlx::query_scalar("SELECT status FROM gateway_secret_leases WHERE invocation_id = $1")
            .bind(invocation)
            .fetch_one(&pool)
            .await
            .expect("read uncommitted lease status");
    assert_eq!(lease_status, "active");
    lock.rollback().await.expect("release authority lock");
}
