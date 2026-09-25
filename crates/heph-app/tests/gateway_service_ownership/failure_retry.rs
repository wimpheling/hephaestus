use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_failure_is_idempotent_and_backoff_survives_restart() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let failures = worker_failure_store().await;
    let owner = GatewayServiceOwner::new("failure-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("claim");
    let startup = GatewayServiceFailure::new(GatewayServiceFailureCode::Startup, None, None)
        .expect("startup failure");
    let invalid_shape = sqlx::query(
        "UPDATE gateway_service_instances
            SET failure_code = NULL, failed_at = NULL, exit_code = 7
          WHERE id = $1",
    )
    .bind(claim.identity.instance_id)
    .execute(&pool)
    .await
    .expect_err("non-exit failure metadata shape must be rejected");
    assert!(
        invalid_shape
            .to_string()
            .contains("gateway_service_instance_exit_failure_check")
    );
    failures
        .record_failure(&claim, &owner, startup)
        .await
        .expect("record startup failure");
    let recorded: (String, i32, Option<time::OffsetDateTime>) = sqlx::query_as(
        "SELECT instance.failure_code, retry.failure_streak, retry.next_retry_at
           FROM gateway_service_instances AS instance
           JOIN gateway_service_retry_state AS retry
             ON retry.gateway_id = instance.gateway_id
            AND retry.revision_id = instance.revision_id
          WHERE instance.id = $1",
    )
    .bind(claim.identity.instance_id)
    .fetch_one(&pool)
    .await
    .expect("failure state");
    assert_eq!(recorded.0, "startup");
    assert_eq!(recorded.1, 1);
    assert!(recorded.2.is_some());
    assert!(
        sqlx::query(
            "UPDATE gateway_service_retry_state
            SET next_retry_at = NULL
          WHERE gateway_id = $1 AND revision_id = $2",
        )
        .bind(fixture.gateway)
        .bind(fixture.revision)
        .execute(&pool)
        .await
        .is_err()
    );

    let duplicate =
        GatewayServiceFailure::new(GatewayServiceFailureCode::UnexpectedExit, Some(17), None)
            .expect("duplicate failure");
    failures
        .record_failure(&claim, &owner, duplicate)
        .await
        .expect("duplicate failure is harmless");
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
    .expect("unchanged failure state");
    assert_eq!(unchanged, (String::from("startup"), 1));
    assert!(matches!(
        ownership
            .claim_new(
                fixture.gateway,
                fixture.revision,
                &owner,
                Duration::from_secs(30),
            )
            .await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));

    let stopping = ownership
        .mark_stopping(&claim, &owner)
        .await
        .expect("stop failed instance");
    ownership
        .mark_cleaned(&stopping, &owner)
        .await
        .expect("clean failed instance");
    let restarted_ownership = worker_ownership().await;
    assert!(matches!(
        restarted_ownership
            .claim_new(
                fixture.gateway,
                fixture.revision,
                &owner,
                Duration::from_secs(30),
            )
            .await,
        Err(GatewayServiceOwnershipError::Conflict)
    ));
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let replacement = restarted_ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("claim after durable backoff");
    let starting = restarted_ownership
        .mark_starting(&replacement, &owner)
        .await
        .expect("starting replacement");
    let ready = restarted_ownership
        .mark_ready(&starting, &owner)
        .await
        .expect("ready replacement");
    let reset: (i32, Option<time::OffsetDateTime>) = sqlx::query_as(
        "SELECT failure_streak, next_retry_at
           FROM gateway_service_retry_state
          WHERE gateway_id = $1 AND revision_id = $2",
    )
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .fetch_one(&pool)
    .await
    .expect("reset retry state");
    assert_eq!(reset, (0, None));
    failures
        .record_failure(
            &ready,
            &owner,
            GatewayServiceFailure::new(GatewayServiceFailureCode::UnexpectedExit, Some(17), None)
                .expect("exit failure"),
        )
        .await
        .expect("record exit failure");
    let exit: (String, Option<i32>, Option<i32>) = sqlx::query_as(
        "SELECT failure_code, exit_code, exit_signal
           FROM gateway_service_instances WHERE id = $1",
    )
    .bind(ready.identity.instance_id)
    .fetch_one(&pool)
    .await
    .expect("stored exit failure");
    assert_eq!(exit, (String::from("unexpected_exit"), Some(17), None));
    let stopping = restarted_ownership
        .mark_stopping(&ready, &owner)
        .await
        .expect("stop replacement");
    restarted_ownership
        .mark_cleaned(&stopping, &owner)
        .await
        .expect("clean replacement");
}
