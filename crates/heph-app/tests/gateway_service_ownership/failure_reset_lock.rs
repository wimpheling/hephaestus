use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_failure_and_ready_reset_recheck_expiry_after_retry_lock() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let failures = worker_failure_store().await;
    let owner = GatewayServiceOwner::new("failure-lock-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(1),
        )
        .await
        .expect("claim");
    let startup = GatewayServiceFailure::new(GatewayServiceFailureCode::Startup, None, None)
        .expect("startup failure");
    failures
        .record_failure(&claim, &owner, startup)
        .await
        .expect("initial failure");
    let mut blocker = pool.begin().await.expect("retry blocker");
    sqlx::query(
        "SELECT gateway_id
           FROM gateway_service_retry_state
          WHERE gateway_id = $1 AND revision_id = $2
          FOR UPDATE",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .execute(&mut *blocker)
    .await
    .expect("lock retry state");
    let duplicate_task = {
        let failures = failures.clone();
        let claim = claim.clone();
        let owner = owner.clone();
        tokio::spawn(async move { failures.record_failure(&claim, &owner, startup).await })
    };
    wait_for_lock_named(&pool, "gateway-failure-test", "gateway_service_retry_state").await;
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    blocker.commit().await.expect("release retry blocker");
    assert!(matches!(
        duplicate_task.await.expect("duplicate task"),
        Err(GatewayServiceFailureStoreError::StaleLease)
    ));
    let unchanged: (String, i32) = sqlx::query_as(
        "SELECT instance.failure_code, retry.failure_streak
           FROM gateway_service_instances AS instance
           JOIN gateway_service_retry_state AS retry
             ON retry.gateway_id = instance.gateway_id
            AND retry.revision_id = instance.revision_id
          WHERE instance.id = $1",
    )
    .bind(claim.identity.instance_id)
    .fetch_one(&pool)
    .await
    .expect("unchanged expired failure");
    assert_eq!(unchanged, (String::from("startup"), 1));

    let second = seed_fixture(&pool, "http.service.v1").await;
    let second_claim = ownership
        .claim_new(
            second.gateway,
            second.revision,
            &owner,
            Duration::from_secs(1),
        )
        .await
        .expect("second claim");
    let second_starting = ownership
        .mark_starting(&second_claim, &owner)
        .await
        .expect("second starting");
    failures
        .record_failure(&second_starting, &owner, startup)
        .await
        .expect("second initial failure");
    let mut reset_blocker = pool.begin().await.expect("reset blocker");
    sqlx::query(
        "SELECT gateway_id
           FROM gateway_service_retry_state
          WHERE gateway_id = $1 AND revision_id = $2
          FOR UPDATE",
    )
    .bind(second.gateway)
    .bind(second.revision)
    .execute(&mut *reset_blocker)
    .await
    .expect("lock reset retry state");
    let reset_task = {
        let ownership = ownership.clone();
        let second_starting = second_starting.clone();
        let owner = owner.clone();
        tokio::spawn(async move { ownership.mark_ready(&second_starting, &owner).await })
    };
    wait_for_lock_named(
        &pool,
        "gateway-ownership-test",
        "gateway_service_retry_state",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    reset_blocker.commit().await.expect("release reset blocker");
    assert!(matches!(
        reset_task.await.expect("reset task"),
        Err(GatewayServiceOwnershipError::StaleLease)
    ));
    let state: (String, String, i32) = sqlx::query_as(
        "SELECT instance.state, instance.failure_code, retry.failure_streak
           FROM gateway_service_instances AS instance
           JOIN gateway_service_retry_state AS retry
             ON retry.gateway_id = instance.gateway_id
            AND retry.revision_id = instance.revision_id
          WHERE instance.id = $1",
    )
    .bind(second_claim.identity.instance_id)
    .fetch_one(&pool)
    .await
    .expect("rolled back reset");
    assert_eq!(
        state,
        (String::from("starting"), String::from("startup"), 1)
    );
}
