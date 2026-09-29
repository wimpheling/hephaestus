use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn exact_resolution_includes_cleaned_and_rejects_wrong_identity() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let owner = GatewayServiceOwner::new("exact-resolution-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("claim");
    let resolved = ownership
        .resolve_exact_instance(claim.identity)
        .await
        .expect("resolve exact claim")
        .expect("exact claim exists");
    assert_eq!(resolved.identity, claim.identity);
    assert_eq!(resolved.fencing_token, claim.fencing_token);

    assert!(
        ownership
            .resolve_exact_instance(GatewayServiceIdentity {
                revision_id: Uuid::new_v4(),
                ..claim.identity
            })
            .await
            .expect("wrong revision is an exact absence")
            .is_none()
    );
    assert!(
        ownership
            .resolve_exact_instance(GatewayServiceIdentity {
                instance_id: Uuid::new_v4(),
                ..claim.identity
            })
            .await
            .expect("missing instance is an exact absence")
            .is_none()
    );
    assert!(matches!(
        ownership
            .resolve_exact_instance(GatewayServiceIdentity {
                instance_id: Uuid::nil(),
                ..claim.identity
            })
            .await,
        Err(GatewayServiceOwnershipError::InvalidArgument)
    ));

    ownership
        .mark_stopping(&claim, &owner)
        .await
        .expect("mark stopping");
    ownership
        .mark_cleaned(&claim, &owner)
        .await
        .expect("mark cleaned");
    let cleaned = ownership
        .resolve_exact_instance(claim.identity)
        .await
        .expect("resolve cleaned claim")
        .expect("cleaned exact row remains queryable");
    assert_eq!(cleaned.state, GatewayServiceInstanceState::Cleaned);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn exact_resolution_serializes_committed_and_rolled_back_epochs() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = Arc::new(worker_ownership().await);
    let old_owner =
        GatewayServiceOwner::new("exact-resolution-barrier", Uuid::new_v4()).expect("old owner");
    let claim = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &old_owner,
            Duration::from_millis(1),
        )
        .await
        .expect("claim");
    wait_until_expired(&pool, claim.identity.instance_id).await;
    let recovery_owner = GatewayServiceOwner::new("exact-resolution-barrier", Uuid::new_v4())
        .expect("recovery owner");

    let mut holder = pool.begin().await.expect("begin gateway barrier");
    sqlx::query("SET application_name = 'gateway-exact-resolution-holder'")
        .execute(&mut *holder)
        .await
        .expect("name gateway barrier");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .expect("gateway barrier pid");
    sqlx::query("SELECT id FROM gateways WHERE id = $1 FOR UPDATE")
        .bind(fixture.gateway)
        .execute(&mut *holder)
        .await
        .expect("hold gateway barrier");
    sqlx::query(
        "UPDATE gateway_service_instances
            SET owner_uuid = $2, fencing_token = $3, state = 'stopping',
                heartbeat_at = clock_timestamp(),
                lease_expires_at = clock_timestamp() + interval '1 millisecond'
          WHERE id = $1",
    )
    .bind(claim.identity.instance_id)
    .bind(recovery_owner.owner_uuid)
    .bind(claim.fencing_token + 1)
    .execute(&mut *holder)
    .await
    .expect("stage committed takeover");

    let resolving = tokio::spawn({
        let ownership = Arc::clone(&ownership);
        let identity = claim.identity;
        async move { ownership.resolve_exact_instance(identity).await }
    });
    wait_for_lock_named(&pool, "gateway-ownership-test", "FROM gateways").await;
    let blocked_by: Vec<i32> = sqlx::query_scalar(
        "SELECT unnest(pg_blocking_pids(pid))
           FROM pg_stat_activity
          WHERE application_name = 'gateway-ownership-test'
            AND state = 'active'
            AND wait_event_type = 'Lock'
            AND query LIKE '%FROM gateways%'
          LIMIT 1",
    )
    .fetch_all(&pool)
    .await
    .expect("inspect exact resolution blocker pid");
    assert!(
        blocked_by.contains(&holder_pid),
        "exact resolver was not blocked by gateway holder {holder_pid}: {blocked_by:?}"
    );
    holder.commit().await.expect("commit staged takeover");
    let committed = timeout(Duration::from_secs(2), resolving)
        .await
        .expect("committed exact resolution completion")
        .expect("committed exact resolution task")
        .expect("resolve committed epoch")
        .expect("committed takeover exists");
    assert_eq!(committed.fencing_token, claim.fencing_token + 1);
    assert_eq!(committed.owner_uuid, recovery_owner.owner_uuid);

    wait_until_expired(&pool, claim.identity.instance_id).await;
    let mut rollback_holder = pool.begin().await.expect("begin rollback barrier");
    sqlx::query("SET application_name = 'gateway-exact-resolution-holder'")
        .execute(&mut *rollback_holder)
        .await
        .expect("name rollback barrier");
    let rollback_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *rollback_holder)
        .await
        .expect("rollback barrier pid");
    sqlx::query("SELECT id FROM gateways WHERE id = $1 FOR UPDATE")
        .bind(fixture.gateway)
        .execute(&mut *rollback_holder)
        .await
        .expect("hold rollback gateway barrier");
    let rolled_owner =
        GatewayServiceOwner::new("exact-resolution-barrier", Uuid::new_v4()).expect("rolled owner");
    sqlx::query(
        "UPDATE gateway_service_instances
            SET owner_uuid = $2, fencing_token = $3, state = 'stopping',
                heartbeat_at = clock_timestamp(),
                lease_expires_at = clock_timestamp() + interval '30 seconds'
          WHERE id = $1",
    )
    .bind(claim.identity.instance_id)
    .bind(rolled_owner.owner_uuid)
    .bind(committed.fencing_token + 1)
    .execute(&mut *rollback_holder)
    .await
    .expect("stage rolled-back takeover");
    let resolving = tokio::spawn({
        let ownership = Arc::clone(&ownership);
        let identity = claim.identity;
        async move { ownership.resolve_exact_instance(identity).await }
    });
    wait_for_lock_named(&pool, "gateway-ownership-test", "FROM gateways").await;
    let rollback_blocked_by: Vec<i32> = sqlx::query_scalar(
        "SELECT unnest(pg_blocking_pids(pid))
           FROM pg_stat_activity
          WHERE application_name = 'gateway-ownership-test'
            AND state = 'active'
            AND wait_event_type = 'Lock'
            AND query LIKE '%FROM gateways%'
          LIMIT 1",
    )
    .fetch_all(&pool)
    .await
    .expect("inspect rollback resolution blocker pid");
    assert!(
        rollback_blocked_by.contains(&rollback_pid),
        "exact resolver was not blocked by rollback holder {rollback_pid}: {rollback_blocked_by:?}"
    );
    rollback_holder
        .rollback()
        .await
        .expect("rollback staged newer epoch");
    let resolved = timeout(Duration::from_secs(2), resolving)
        .await
        .expect("exact resolution completion")
        .expect("exact resolution task")
        .expect("resolve rolled-back epoch")
        .expect("original committed epoch remains");
    assert_eq!(resolved.fencing_token, committed.fencing_token);
    assert_eq!(resolved.owner_uuid, recovery_owner.owner_uuid);
}
