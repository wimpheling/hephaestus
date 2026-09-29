use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_failure_requires_exact_live_fence_and_redacted_exit_shape() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed_fixture(&pool, "http.service.v1").await;
    let ownership = worker_ownership().await;
    let failures = worker_failure_store().await;
    let owner = GatewayServiceOwner::new("failure-fence-host", Uuid::new_v4()).expect("owner");
    let claim = ownership
        .claim_new(
            fixture.gateway,
            fixture.revision,
            &owner,
            Duration::from_secs(30),
        )
        .await
        .expect("claim");
    let mut wrong_fence = claim.clone();
    wrong_fence.fencing_token += 1;
    let failure = GatewayServiceFailure::new(GatewayServiceFailureCode::Startup, None, None)
        .expect("failure");
    assert!(matches!(
        failures.record_failure(&wrong_fence, &owner, failure).await,
        Err(GatewayServiceFailureStoreError::StaleLease)
    ));
    let wrong_owner =
        GatewayServiceOwner::new("failure-fence-host", Uuid::new_v4()).expect("owner");
    assert!(matches!(
        failures.record_failure(&claim, &wrong_owner, failure).await,
        Err(GatewayServiceFailureStoreError::StaleLease)
    ));
    let expired_fixture = seed_fixture(&pool, "http.service.v1").await;
    let expired_claim = ownership
        .claim_new(
            expired_fixture.gateway,
            expired_fixture.revision,
            &owner,
            Duration::from_millis(5),
        )
        .await
        .expect("short-lived claim");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(matches!(
        failures
            .record_failure(&expired_claim, &owner, failure)
            .await,
        Err(GatewayServiceFailureStoreError::StaleLease)
    ));
    assert!(
        GatewayServiceFailure::new(GatewayServiceFailureCode::Startup, Some(1), None,).is_err()
    );
    assert!(
        GatewayServiceFailure::new(GatewayServiceFailureCode::UnexpectedExit, Some(256), None,)
            .is_err()
    );
    let signal =
        GatewayServiceFailure::new(GatewayServiceFailureCode::UnexpectedExit, None, Some(9))
            .expect("bounded signal");
    assert_eq!(signal.exit_signal, Some(9));
}
